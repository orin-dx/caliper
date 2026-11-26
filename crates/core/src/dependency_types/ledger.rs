//! The layer's one record of the dependency files it could not read, and of what those reads left unfollowed.
//!
//! A read that fails is unknown, not absent ([`ReadFailed`]). Every decision that depends on one asks this ledger: the
//! warnings a layer reports (one per file whose latest read failed), which demands a load follows again, which answers a
//! lazy session may memoize, and whether a cold load may write the index. Nothing else holds the fact.
//!
//! Reads are keyed by path, and marks by the node of the walk that met the failure: a demand, or a declaration it
//! claimed. A mark is what a follow found, so a later read of the same file by another walk does not clear it: only
//! following that demand again does. Each layer holds only what changed since the layer below; lookups take the newest
//! entry in the chain, and a flatten merges the chain's ledgers the same way.

use super::*;

/// A declaration a load claimed: `(file, name)`.
pub(super) type ClaimKey = (PathKey, CompactString);

/// A node of the walk a load makes: a demand, or a declaration one claimed, whose references it followed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Node {
    Demand(DemandKey),
    Claim(ClaimKey),
}

/// The latest read of one dependency file in the chain, kept only once a read of it failed.
#[derive(Clone, Debug)]
enum Read {
    /// Read, or found missing, after an earlier read failed: what it declares is known again.
    Done,
    /// The file exists but could not be read; the warning reports it.
    Failed(Diagnostic),
}

/// What the latest follow of a node left unread. A node with no mark in the chain reached nothing unread.
#[derive(Clone, Debug, Default)]
struct Mark {
    /// The demand's own walk passed a file it could not read: following it again may answer more.
    unread: Option<Demand>,
    /// The marked nodes it reached: below it in the same load, or marked by an earlier one.
    reaches: Box<[Node]>,
}

impl Mark {
    fn is_clean(&self) -> bool {
        self.unread.is_none() && self.reaches.is_empty()
    }
}

/// One layer's entries: what its load read again or could not read, and the marks its follows left.
#[derive(Default)]
pub(super) struct ReadLedger {
    reads: FxHashMap<PathKey, Read>,
    marks: FxHashMap<Node, Mark>,
}

impl ReadLedger {
    /// `newer`'s entries over this ledger's, as a flatten merges a chain: the latest entry wins.
    fn absorb(&mut self, newer: &ReadLedger) {
        self.reads.extend(newer.reads.iter().map(|(path, read)| (path.clone(), read.clone())));
        self.marks.extend(newer.marks.iter().map(|(node, mark)| (node.clone(), mark.clone())));
    }
}

/// The demands whose own walk is unread, among `start` and the nodes their marks reach, sorted.
fn unread_below<'a>(start: impl IntoIterator<Item = Node>, mark: impl Fn(&Node) -> Option<&'a Mark>) -> Vec<Demand> {
    let mut found: Vec<Demand> = Vec::new();
    let mut seen: FxHashSet<Node> = FxHashSet::default();
    let mut stack: Vec<Node> = start.into_iter().collect();
    while let Some(node) = stack.pop() {
        let Some(marked) = mark(&node) else { continue };
        if !seen.insert(node) {
            continue;
        }
        found.extend(marked.unread.iter().cloned());
        stack.extend(marked.reaches.iter().cloned());
    }
    found.sort();
    found.dedup();
    found
}

impl DependencyLayer {
    fn has_marks(&self) -> bool {
        self.chain().any(|layer| !layer.ledger.marks.is_empty())
    }

    fn latest_read(&self, file: &str) -> Option<&Read> {
        self.chain().find_map(|layer| layer.ledger.reads.get(file))
    }

    fn mark(&self, node: &Node) -> Option<&Mark> {
        self.chain().find_map(|layer| layer.ledger.marks.get(node))
    }

    /// Whether the latest read of `file` in the chain failed.
    fn read_failed(&self, file: &str) -> bool {
        matches!(self.latest_read(file), Some(Read::Failed(_)))
    }

    /// One warning for each file whose latest read in the chain failed, sorted by message.
    pub(crate) fn read_warnings(&self) -> Vec<Diagnostic> {
        let mut seen: FxHashSet<&PathKey> = FxHashSet::default();
        let mut warnings: Vec<Diagnostic> = Vec::new();
        for layer in self.chain() {
            for (path, read) in &layer.ledger.reads {
                if seen.insert(path)
                    && let Read::Failed(warning) = read
                {
                    warnings.push(warning.clone());
                }
            }
        }
        warnings.sort_by(|a, b| (&a.message, &a.file).cmp(&(&b.message, &b.file)));
        warnings
    }

    /// Whether the demand's own walk passed a file it could not read, the last time the chain followed it.
    pub(super) fn awaits_retry(&self, key: &DemandKey) -> bool {
        self.has_marks() && self.mark(&Node::Demand(key.clone())).is_some_and(|mark| mark.unread.is_some())
    }

    /// Whether a walk of the chain passed a file it could not read and has not been followed since.
    pub(crate) fn any_unread(&self) -> bool {
        self.chain()
            .flat_map(|layer| layer.ledger.marks.iter())
            .any(|(node, mark)| mark.unread.is_some() && self.mark(node).is_some_and(|latest| latest.unread.is_some()))
    }

    /// The demands that `roots` lead to, themselves included, whose walk passed a file that could not be read and has
    /// not been followed since, sorted. Following them again is what may answer more; while there are any, what was
    /// resolved from `roots` holds for one request only.
    pub(crate) fn unread(&self, roots: &[Demand]) -> Vec<Demand> {
        if !self.has_marks() {
            return Vec::new();
        }
        unread_below(roots.iter().map(|demand| Node::Demand(demand.key())), |node| self.mark(node))
    }
}

/// The ledger one load builds as it reads and walks; [`LedgerBuilder::finish`] makes the layer's.
pub(super) struct LedgerBuilder<'a> {
    parent: Option<&'a DependencyLayer>,
    /// The chain a flatten took in, merged: consulted as a parent is.
    inherited: ReadLedger,
    reads: FxDashMap<PathKey, Read>,
    /// Each node this load followed, with the nodes it reached: the demands it spawned or met, the declarations it
    /// claimed or met claimed.
    edges: FxDashMap<Node, Vec<Node>>,
    /// The demands whose own walk passed a file it could not read.
    unread: FxDashMap<DemandKey, Demand>,
    /// The demands followed again because their last walk was unread.
    retried: FxDashSet<DemandKey>,
}

impl<'a> LedgerBuilder<'a> {
    pub(super) fn new(parent: Option<&'a DependencyLayer>) -> Self {
        LedgerBuilder {
            parent,
            inherited: ReadLedger::default(),
            reads: DashMap::default(),
            edges: DashMap::default(),
            unread: DashMap::default(),
            retried: DashSet::default(),
        }
    }

    /// Take in a layer of the chain a flatten replaces; called oldest first.
    pub(super) fn inherit(&mut self, layer: &ReadLedger) {
        self.inherited.absorb(layer);
    }

    fn earlier_mark(&self, node: &Node) -> Option<&Mark> {
        self.inherited.marks.get(node).or_else(|| self.parent.and_then(|parent| parent.mark(node)))
    }

    fn has_earlier_marks(&self) -> bool {
        !self.inherited.marks.is_empty() || self.parent.is_some_and(DependencyLayer::has_marks)
    }

    /// `file` was read, or found missing. Recorded only over an earlier failure, which it ends; within one load a
    /// failure stands, since the walk that met it is unread.
    pub(super) fn read_done(&self, file: &Utf8Path) {
        let earlier = match self.inherited.reads.get(file.as_str()) {
            Some(read) => matches!(read, Read::Failed(_)),
            None => self.parent.is_some_and(|parent| parent.read_failed(file.as_str())),
        };
        if earlier {
            self.reads.entry(PathKey::from(file)).or_insert(Read::Done);
        }
    }

    pub(super) fn read_failed(&self, file: &Utf8Path, warning: Diagnostic) {
        self.reads.insert(PathKey::from(file), Read::Failed(warning));
    }

    /// Whether this load had a read fail: what it holds is not the whole answer, so it writes no index.
    pub(super) fn any_failed(&self) -> bool {
        self.reads.iter().any(|entry| matches!(entry.value(), Read::Failed(_)))
    }

    /// `from` reached `to`, whether it followed `to` itself or found it followed already.
    pub(super) fn reached(&self, from: &Node, to: Node) {
        self.edges.entry(from.clone()).or_default().push(to);
    }

    pub(super) fn unread_demand(&self, demand: &Demand) {
        self.unread.insert(demand.key(), demand.clone());
    }

    /// Whether a demand followed before this load is to be followed again (its last walk was unread), and this is the
    /// first time this load meets it.
    pub(super) fn retries(&self, key: &DemandKey) -> bool {
        let unread = self.has_earlier_marks()
            && self.earlier_mark(&Node::Demand(key.clone())).is_some_and(|mark| mark.unread.is_some());
        unread && self.retried.insert(key.clone())
    }

    /// The layer's ledger. A node this load followed is marked when it reached an unread walk: its own, one below it
    /// in this load, or one an earlier load marked that has not been followed since. A demand followed again that
    /// reached none is marked clean, over its earlier mark. A ledger with no parent stands alone, so the chain it took
    /// in is merged into it and what is clean is dropped.
    pub(super) fn finish(self) -> ReadLedger {
        let has_earlier = self.has_earlier_marks();
        let LedgerBuilder { parent, inherited, reads, edges, unread, retried } = self;
        let earlier = |node: &Node| inherited.marks.get(node).or_else(|| parent.and_then(|parent| parent.mark(node)));
        let unread: FxHashMap<Node, Demand> =
            unread.into_iter().map(|(key, demand)| (Node::Demand(key), demand)).collect();
        let mut marks: FxHashMap<Node, Mark> = FxHashMap::default();
        if !unread.is_empty() || has_earlier {
            let edges: FxHashMap<Node, Vec<Node>> = edges.into_iter().collect();
            let followed = |node: &Node| {
                edges.contains_key(node)
                    || unread.contains_key(node)
                    || matches!(node, Node::Demand(key) if retried.contains(key))
            };
            // The walks that are unread: this load's own, and the earlier ones this load met without following.
            let mut marked: FxHashSet<&Node> = unread.keys().collect();
            let mut queue: Vec<&Node> = marked.iter().copied().collect();
            let mut reached_by: FxHashMap<&Node, Vec<&Node>> = FxHashMap::default();
            for (from, tos) in &edges {
                for to in tos {
                    reached_by.entry(to).or_default().push(from);
                    if !followed(to)
                        && !marked.contains(to)
                        && earlier(to).is_some()
                        && !unread_below([to.clone()], earlier).is_empty()
                    {
                        marked.insert(to);
                        queue.push(to);
                    }
                }
            }
            while let Some(node) = queue.pop() {
                for &from in reached_by.get(node).into_iter().flatten() {
                    if marked.insert(from) {
                        queue.push(from);
                    }
                }
            }
            for &node in marked.iter().filter(|node| followed(node)) {
                let reaches = edges.get(node).into_iter().flatten().filter(|to| marked.contains(to)).cloned().collect();
                marks.insert(node.clone(), Mark { unread: unread.get(node).cloned(), reaches });
            }
            for key in retried {
                marks.entry(Node::Demand(key)).or_default();
            }
        }
        let reads: FxHashMap<PathKey, Read> = reads.into_iter().collect();
        if parent.is_some() {
            return ReadLedger { reads, marks };
        }
        let mut merged = inherited;
        merged.absorb(&ReadLedger { reads, marks });
        merged.reads.retain(|_, read| matches!(read, Read::Failed(_)));
        merged.marks.retain(|_, mark| !mark.is_clean());
        merged
    }
}

#[cfg(test)]
mod tests;
