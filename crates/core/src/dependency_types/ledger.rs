//! The layer's one record of the dependency files it could not read, and of what those reads left unfollowed.
//!
//! A read that fails is unknown, not absent ([`ReadFailed`]). Every decision that depends on one asks this ledger: the
//! warnings a layer reports (one per file whose latest read failed), which demands a load follows again, which answers
//! a lazy session may memoize, and whether a cold load may write the index. Nothing else holds the fact.
//!
//! Reads are keyed by path, and marks by the node of the walk that met the failure: a demand, or a declaration it
//! claimed. A mark is what a follow found, so a later read of the same file by another walk does not clear it: only
//! following that demand again does. Each layer holds only what changed since the layer below; lookups take the newest
//! entry in the chain, and a flatten merges the chain's ledgers the same way.
//!
//! A file that stays unreadable (permissions, a broken install) would otherwise make every request follow its walk
//! again, and pay a load and a resolution for it. A lazy session follows one again only once it is due ([`Retry`]): a
//! file its own walk could not read was read since, [`DEGRADED_RETRY`] passed since that read failed, or an edit
//! arrived. Deciding that touches no file: a file that changes on disk is read again at the retry or the next edit.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::*;

/// How long a result degraded by a transient failure stands before it is asked for again, unless an edit comes first:
/// a walk that passed a dependency file it could not read, and a request the checker answered with nothing. Either
/// would otherwise be retried by every request, each paying a load or a checker restart to fail again; one that
/// recovered answers within this.
pub(crate) const DEGRADED_RETRY: Duration = Duration::from_secs(30);

/// How many edits a session has seen. A failed read stamped with fewer than the session's count is due at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Edits(u64);

impl Edits {
    /// The count once one more edit arrived.
    pub(crate) fn next(self) -> Edits {
        Edits(self.0 + 1)
    }
}

/// Which of a session's loads made a read, so a [`Footprint`] tells one failure of a file from a later one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Load(u64);

/// A session's source of [`Load`]s, shared by its revisions.
#[derive(Debug, Default)]
pub(crate) struct LoadIds(AtomicU64);

impl LoadIds {
    pub(crate) fn next(&self) -> Load {
        Load(self.0.fetch_add(1, Ordering::Relaxed))
    }
}

/// A moment of a session: its time and how many edits it had seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Moment {
    pub(crate) at: Instant,
    pub(crate) edits: Edits,
}

/// What a load records each failed read with: the session's moment and the load's own id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) moment: Moment,
    pub(crate) load: Load,
}

impl Stamp {
    /// The stamp of a load no session compares: a batch run, or an eager revision, which follows every walk on each
    /// change. A lazy session stamps its own loads, its first one included.
    pub(crate) fn unsessioned() -> Stamp {
        Stamp { moment: Moment { at: Instant::now(), edits: Edits::default() }, load: Load(0) }
    }
}

/// When a load follows again a walk that passed a file it could not read.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Retry {
    /// Every such walk: a caller that follows on each change (watch's eager revisions, a cold run).
    Always,
    /// Once it is due at this moment: a file its walk could not read was read since, [`DEGRADED_RETRY`] passed since
    /// that read failed, or an edit arrived since.
    Due(Moment),
}

/// When a read failed, by the session's clock at the failure, not at the start of the load that made it: a walk past
/// it is due again [`DEGRADED_RETRY`] after this, however long the load ran first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FailedAt(Instant);

/// The session's time, read when a read fails.
pub(crate) type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// A read that failed: the stamp of the load that made it, and when it failed.
#[derive(Clone, Debug)]
struct FailedRead {
    warning: Diagnostic,
    stamp: Stamp,
    at: FailedAt,
}

impl FailedRead {
    /// When a walk that met this failure is due again by time; `None` when an edit since already made it due.
    fn retry_at(&self, edits: Edits) -> Option<Instant> {
        (self.stamp.moment.edits >= edits).then(|| self.at.0 + DEGRADED_RETRY)
    }
}

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
    Failed(FailedRead),
}

/// What the latest follow of a node left unread. A node with no mark in the chain reached nothing unread.
#[derive(Clone, Debug, Default)]
struct Mark {
    /// The demand's own walk passed a file it could not read: following it again may answer more.
    unread: Option<Demand>,
    /// The files the demand's own walk could not read, the last time it was followed.
    suspects: Arc<[PathKey]>,
    /// The marked nodes it reached: below it in the same load, or marked by an earlier one.
    reaches: Box<[Node]>,
}

impl Mark {
    fn is_clean(&self) -> bool {
        self.unread.is_none() && self.reaches.is_empty()
    }

    /// Whether a load under `retry` follows this mark's demand again: its own walk is unread and that is due.
    fn is_due<'a>(&self, retry: Retry, latest: impl Fn(&str) -> Option<&'a Read>) -> bool {
        if self.unread.is_none() {
            return false;
        }
        let Retry::Due(now) = retry else { return true };
        self.retry_at(now.edits, latest).is_none_or(|at| at <= now.at)
    }

    /// When this mark's unread walk is due by time, the earliest of its files' retries; `None` when it is due already
    /// whatever the time: an edit arrived since a failure, or one of its files was read since.
    fn retry_at<'a>(&self, edits: Edits, latest: impl Fn(&str) -> Option<&'a Read>) -> Option<Instant> {
        let mut earliest: Option<Instant> = None;
        for path in self.suspects.iter() {
            let at = match latest(std::borrow::Borrow::borrow(path)) {
                Some(Read::Failed(failed)) => failed.retry_at(edits)?,
                Some(Read::Done) | None => return None,
            };
            earliest = Some(earliest.map_or(at, |earliest| earliest.min(at)));
        }
        earliest
    }
}

/// The walks of a chain that are unread, and what reached them: what was read through these may lack what the files
/// they could not read hold ([`DependencyLayer::unread_walks`]).
#[derive(Default)]
pub(crate) struct UnreadWalks {
    demands: FxHashSet<DemandKey>,
    /// Every node of the chain whose latest follow reached an unread walk: what a load over this layer asks of each
    /// earlier node it meets.
    nodes: FxHashSet<Node>,
    /// The directories of `demands`, so a project file whose directory none is in costs no key.
    dirs: FxHashSet<Box<str>>,
    /// The files of the declarations whose references reach an unread walk.
    files: FxHashSet<PathKey>,
    /// The files whose latest read failed.
    failed: FxHashSet<PathKey>,
}

impl UnreadWalks {
    pub(crate) fn is_empty(&self) -> bool {
        self.demands.is_empty() && self.files.is_empty() && self.failed.is_empty()
    }

    /// Whether one of `demands` (a project file's) reached an unread walk: what was read through it may lack what that
    /// walk would have given.
    pub(crate) fn reached_by(&self, demands: &[Demand]) -> bool {
        demands.iter().any(|demand| {
            self.dirs.contains(demand.from.parent().map_or("", Utf8Path::as_str))
                && self.demands.contains(&demand.key())
        })
    }

    /// Whether a declaration in the package file `file` reached an unread walk.
    pub(crate) fn declared_in(&self, file: &str) -> bool {
        self.files.contains(file)
    }

    /// Whether what was read in `file` may change once a walk that could not read a file is followed again: a
    /// declaration in it reached an unread walk, or it is itself a file whose latest read failed.
    pub(crate) fn touches(&self, file: &str) -> bool {
        self.files.contains(file) || self.failed.contains(file)
    }
}

/// What a provisional answer read through unread walks ([`DependencyLayer::footprint`]): each file those walks could
/// not read, with the failed read the answer saw. The one rule for every answer resolved past a file that could not be
/// read: it holds while each of these is still that failed read and not due ([`DependencyLayer::footprint_holds`]),
/// and not a moment after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Footprint(Box<[Unreadable]>);

/// A file an answer's walks could not read, and the stamp and time of the failed read the answer saw.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unreadable {
    path: PathKey,
    read: Stamp,
    at: FailedAt,
}

/// One layer's entries: what its load read again or could not read, and the marks its follows left.
#[derive(Default)]
pub(super) struct ReadLedger {
    reads: FxHashMap<PathKey, Read>,
    marks: FxHashMap<Node, Mark>,
    /// How many earlier nodes `finish` walked the ledger from to learn whether they reach an unread walk, for tests.
    #[cfg(test)]
    traversals: usize,
}

impl ReadLedger {
    /// Whether this ledger's latest read of `file` failed.
    pub(super) fn failed(&self, file: &str) -> bool {
        matches!(self.reads.get(file), Some(Read::Failed(_)))
    }

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
                    && let Read::Failed(failed) = read
                {
                    warnings.push(failed.warning.clone());
                }
            }
        }
        warnings.sort_by(|a, b| (&a.message, &a.file).cmp(&(&b.message, &b.file)));
        warnings
    }

    /// Whether the demand's own walk passed a file it could not read, the last time the chain followed it, and a load
    /// under `retry` follows it again.
    pub(super) fn awaits_retry(&self, key: &DemandKey, retry: Retry) -> bool {
        self.has_marks()
            && self
                .mark(&Node::Demand(key.clone()))
                .is_some_and(|mark| mark.is_due(retry, |path| self.latest_read(path)))
    }

    /// The footprint of an answer resolved through the unread walks `unread` ([`Self::unread`]): every file those
    /// walks could not read, with the failed read the answer saw. `None` when one of them is no longer unread.
    pub(crate) fn footprint(&self, unread: &[Demand]) -> Option<Footprint> {
        let mut files: Vec<Unreadable> = Vec::new();
        for demand in unread {
            let mark = self.mark(&Node::Demand(demand.key())).filter(|mark| mark.unread.is_some())?;
            for path in mark.suspects.iter() {
                let Some(Read::Failed(failed)) = self.latest_read(std::borrow::Borrow::borrow(path)) else {
                    return None;
                };
                files.push(Unreadable { path: path.clone(), read: failed.stamp, at: failed.at });
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        (!files.is_empty()).then(|| Footprint(files.into_boxed_slice()))
    }

    /// Whether an answer with `footprint` still holds: every file in it is still the failed read the answer saw (no
    /// load read it since, whichever walk), its retry has not passed, and no edit arrived since.
    pub(crate) fn footprint_holds(&self, footprint: &Footprint, now: Moment) -> bool {
        footprint.0.iter().all(|Unreadable { path, read, at }| {
            let same = matches!(self.latest_read(std::borrow::Borrow::borrow(path)),
                Some(Read::Failed(failed)) if failed.stamp == *read && failed.at == *at);
            same && read.moment.edits >= now.edits && now.at < at.0 + DEGRADED_RETRY
        })
    }

    /// How many distinct lists of files this layer's own unread walks hold, for tests.
    #[cfg(test)]
    pub(crate) fn distinct_suspect_lists(&self) -> usize {
        let lists: FxHashSet<*const [PathKey]> = self
            .ledger
            .marks
            .values()
            .filter(|mark| mark.unread.is_some())
            .map(|mark| Arc::as_ptr(&mark.suspects))
            .collect();
        lists.len()
    }

    /// How many files each of this layer's own unread walks holds, for tests.
    #[cfg(test)]
    pub(crate) fn suspect_counts(&self) -> Vec<usize> {
        self.ledger.marks.values().filter(|mark| mark.unread.is_some()).map(|mark| mark.suspects.len()).collect()
    }

    /// Every node of the chain whose latest follow reached a walk that is unread, its own or one below it. Built once
    /// per layer, and not at all for a layer whose load changed no entry of the ledger (a growth that met no unread
    /// walk): that layer's are its parent's.
    pub(crate) fn unread_walks(&self) -> Arc<UnreadWalks> {
        self.unread_walks
            .get_or_init(|| match &self.parent {
                Some(parent) if self.ledger.reads.is_empty() && self.ledger.marks.is_empty() => parent.unread_walks(),
                Some(_) | None => {
                    #[cfg(test)]
                    self.unread_walk_builds.fetch_add(1, Ordering::Relaxed);
                    Arc::new(self.unread_walks_counted().0)
                }
            })
            .clone()
    }

    /// How many times [`Self::unread_walks`] was built across the chain, for tests.
    #[cfg(test)]
    pub(crate) fn unread_walk_builds(&self) -> usize {
        self.chain().map(|layer| layer.unread_walk_builds.load(Ordering::Relaxed)).sum()
    }

    /// [`Self::unread_walks`], and how many nodes its traversal visited: one reverse traversal from the unread walks
    /// over the latest marks, each node once, so a chain of N declarations ending at one unread file costs N.
    pub(crate) fn unread_walks_counted(&self) -> (UnreadWalks, usize) {
        let mut walks = UnreadWalks::default();
        if !self.has_marks() {
            return (walks, 0);
        }
        let mut latest: FxHashMap<&Node, &Mark> = FxHashMap::default();
        for layer in self.chain() {
            for (node, mark) in &layer.ledger.marks {
                latest.entry(node).or_insert(mark);
            }
            for (path, read) in &layer.ledger.reads {
                if self.latest_read(std::borrow::Borrow::borrow(path)).is_some_and(|latest| std::ptr::eq(latest, read))
                    && matches!(read, Read::Failed(_))
                {
                    walks.failed.insert(path.clone());
                }
            }
        }
        let mut reached_by: FxHashMap<&Node, Vec<&Node>> = FxHashMap::default();
        let mut marked: FxHashSet<&Node> = FxHashSet::default();
        let mut queue: Vec<&Node> = Vec::new();
        for (&node, mark) in &latest {
            for to in mark.reaches.iter() {
                reached_by.entry(to).or_default().push(node);
            }
            if mark.unread.is_some() && marked.insert(node) {
                queue.push(node);
            }
        }
        let mut visits = 0;
        while let Some(node) = queue.pop() {
            visits += 1;
            for &from in reached_by.get(node).into_iter().flatten() {
                if marked.insert(from) {
                    queue.push(from);
                }
            }
        }
        for node in marked {
            walks.nodes.insert(node.clone());
            match node {
                Node::Demand(key) => {
                    walks.dirs.insert(key.0.clone());
                    walks.demands.insert(key.clone())
                }
                Node::Claim((file, _)) => walks.files.insert(file.clone()),
            };
        }
        (walks, visits)
    }

    /// Every demand of the chain whose own walk is unread and that a load under `retry` follows again, sorted. An edit
    /// makes each such walk due ([`Mark::is_due`]), whatever file it was: the eager revision follows these on every
    /// edit.
    pub(crate) fn due_walks(&self, retry: Retry) -> Vec<Demand> {
        if !self.has_marks() {
            return Vec::new();
        }
        let mut seen: FxHashSet<&Node> = FxHashSet::default();
        let mut due: Vec<Demand> = Vec::new();
        for layer in self.chain() {
            for (node, mark) in &layer.ledger.marks {
                // The newest mark of a node is its latest follow; an older one below it is superseded.
                if seen.insert(node)
                    && let Some(demand) = &mark.unread
                    && mark.is_due(retry, |path| self.latest_read(path))
                {
                    due.push(demand.clone());
                }
            }
        }
        due.sort();
        due.dedup();
        due
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
    /// The demands whose own walk passed a file it could not read, with those files.
    unread: FxDashMap<DemandKey, (Demand, Arc<[PathKey]>)>,
    /// One copy of each list of files a walk could not read: the demands of one augmentation search (every directory
    /// of a resolution scope importing the module) share the search's list instead of each holding its own.
    suspect_lists: FxDashMap<Arc<[PathKey]>, ()>,
    /// The demands followed again because their last walk was unread.
    retried: FxDashSet<DemandKey>,
    /// When this load follows again a demand an earlier one left unread.
    pub(super) retry: Retry,
    /// What its failed reads are recorded with.
    pub(super) stamp: Stamp,
    /// The session's time, read at each failure.
    pub(super) clock: Clock,
}

impl<'a> LedgerBuilder<'a> {
    pub(super) fn new(parent: Option<&'a DependencyLayer>) -> Self {
        LedgerBuilder {
            retry: Retry::Always,
            stamp: Stamp::unsessioned(),
            clock: Arc::new(Instant::now),
            parent,
            inherited: ReadLedger::default(),
            reads: DashMap::default(),
            edges: DashMap::default(),
            unread: DashMap::default(),
            suspect_lists: DashMap::default(),
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
        let at = FailedAt((self.clock)());
        self.reads.insert(PathKey::from(file), Read::Failed(FailedRead { warning, stamp: self.stamp, at }));
    }

    /// `from` reached `to`, whether it followed `to` itself or found it followed already.
    pub(super) fn reached(&self, from: &Node, to: Node) {
        self.edges.entry(from.clone()).or_default().push(to);
    }

    /// `demand`'s own walk could not read `files`.
    pub(super) fn unread_demand(&self, demand: &Demand, mut files: Vec<PathKey>) {
        files.sort();
        files.dedup();
        // One entry call, so two threads with the same list keep one copy.
        let shared = match self.suspect_lists.entry(Arc::from(files)) {
            dashmap::mapref::entry::Entry::Occupied(held) => held.key().clone(),
            dashmap::mapref::entry::Entry::Vacant(new) => {
                let list = new.key().clone();
                new.insert(());
                list
            }
        };
        self.unread.insert(demand.key(), (demand.clone(), shared));
    }

    /// Whether a demand followed before this load is to be followed again (its last walk was unread and the retry is
    /// due), and this is the first time this load meets it.
    pub(super) fn retries(&self, key: &DemandKey) -> bool {
        let latest = |path: &str| {
            self.inherited.reads.get(path).or_else(|| self.parent.and_then(|parent| parent.latest_read(path)))
        };
        let due = self.has_earlier_marks()
            && self.earlier_mark(&Node::Demand(key.clone())).is_some_and(|mark| mark.is_due(self.retry, latest));
        due && self.retried.insert(key.clone())
    }

    /// The layer's ledger. A node this load followed is marked when it reached an unread walk: its own, one below it
    /// in this load, or one an earlier load marked that has not been followed since. A demand followed again that
    /// reached none is marked clean, over its earlier mark. A ledger with no parent stands alone, so the chain it took
    /// in is merged into it and what is clean is dropped.
    pub(super) fn finish(self) -> ReadLedger {
        let has_earlier = self.has_earlier_marks();
        let LedgerBuilder {
            parent,
            inherited,
            reads,
            edges,
            unread,
            retried,
            retry: _,
            stamp: _,
            clock: _,
            suspect_lists: _,
        } = self;
        let reads: FxHashMap<PathKey, Read> = reads.into_iter().collect();
        let earlier = |node: &Node| inherited.marks.get(node).or_else(|| parent.and_then(|parent| parent.mark(node)));
        let unread: FxHashMap<Node, (Demand, Arc<[PathKey]>)> =
            unread.into_iter().map(|(key, walk)| (Node::Demand(key), walk)).collect();
        let mut marks: FxHashMap<Node, Mark> = FxHashMap::default();
        #[cfg(test)]
        let traversals = std::cell::Cell::new(0);
        if !unread.is_empty() || has_earlier {
            let edges: FxHashMap<Node, Vec<Node>> = edges.into_iter().collect();
            let followed = |node: &Node| {
                edges.contains_key(node)
                    || unread.contains_key(node)
                    || matches!(node, Node::Demand(key) if retried.contains(key))
            };
            // The walks that are unread: this load's own, and the earlier ones this load met without following. Over a
            // parent, whether an earlier node reaches one is the parent's cached answer, asked once per layer; only a
            // flatten, whose earlier marks it took in itself, walks from each.
            let below = parent.filter(|_| inherited.marks.is_empty()).map(DependencyLayer::unread_walks);
            let reaches_earlier = |node: &Node| match &below {
                Some(walks) => walks.nodes.contains(node),
                None => {
                    #[cfg(test)]
                    traversals.set(traversals.get() + 1);
                    earlier(node).is_some() && !unread_below([node.clone()], earlier).is_empty()
                }
            };
            let mut marked: FxHashSet<&Node> = unread.keys().collect();
            let mut queue: Vec<&Node> = marked.iter().copied().collect();
            let mut reached_by: FxHashMap<&Node, Vec<&Node>> = FxHashMap::default();
            for (from, tos) in &edges {
                for to in tos {
                    reached_by.entry(to).or_default().push(from);
                    if !followed(to) && !marked.contains(to) && reaches_earlier(to) {
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
                let (own, suspects) = match unread.get(node) {
                    Some((demand, files)) => (Some(demand.clone()), files.clone()),
                    None => (None, Arc::default()),
                };
                marks.insert(node.clone(), Mark { unread: own, suspects, reaches });
            }
            for key in retried {
                marks.entry(Node::Demand(key)).or_default();
            }
        }
        if parent.is_some() {
            return ReadLedger {
                reads,
                marks,
                #[cfg(test)]
                traversals: traversals.get(),
            };
        }
        let mut merged = inherited;
        merged.absorb(&ReadLedger {
            reads,
            marks,
            #[cfg(test)]
            traversals: 0,
        });
        #[cfg(test)]
        {
            merged.traversals = traversals.get();
        }
        merged.reads.retain(|_, read| matches!(read, Read::Failed(_)));
        merged.marks.retain(|_, mark| !mark.is_clean());
        merged
    }
}

#[cfg(test)]
mod tests;
