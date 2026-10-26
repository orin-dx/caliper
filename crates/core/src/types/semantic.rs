//! What the experimental TypeScript checker backend hands back, and how it merges over the structural result.
//!
//! The backend (`caliper-docgen-semantic`) owns the process, the protocol and the checker walk; this module owns only
//! the result shape and the merge rules of `docs/type-checker-integration.md`, so `crates/core` stays free of I/O.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use camino::Utf8PathBuf;
use rustc_hash::{FxHashMap, FxHasher};

use crate::types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
use crate::types::output::{ComponentEntry, ExtractionOutput, ParsedProp, PropType};

/// One prop as the checker reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticProp {
    pub name: String,
    pub prop_type: PropType,
    pub required: bool,
    pub description: String,
}

impl SemanticProp {
    fn structural_hash(&self) -> u64 {
        let mut hasher = FxHasher::default();
        self.name.hash(&mut hasher);
        self.required.hash(&mut hasher);
        self.description.hash(&mut hasher);
        self.prop_type.structural_hash(&mut hasher);
        hasher.finish()
    }
}

/// Shares identical props between components, as the structural resolver does: a design system's components repeat
/// the same few hundred props, and Chakra's 600 components would otherwise hold 470,000 copies.
#[derive(Debug, Default)]
pub struct PropInterner {
    by_hash: Mutex<FxHashMap<u64, Vec<Arc<SemanticProp>>>>,
}

impl PropInterner {
    pub fn share(&self, prop: SemanticProp) -> Arc<SemanticProp> {
        let hash = prop.structural_hash();
        let Ok(mut by_hash) = self.by_hash.lock() else { return Arc::new(prop) };
        let bucket = by_hash.entry(hash).or_default();
        if let Some(existing) = bucket.iter().find(|existing| ***existing == prop) {
            return existing.clone();
        }
        let shared = Arc::new(prop);
        bucket.push(shared.clone());
        shared
    }
}

/// One component the checker answered for, found by the file and name the structural pass reported.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticComponent {
    pub file_path: Utf8PathBuf,
    pub display_name: String,
    /// Declared props in declaration order, without anything `@types/react` declares.
    pub props: Vec<Arc<SemanticProp>>,
}

/// A component the backend left to the structural result, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticFallback {
    pub file_path: Utf8PathBuf,
    pub display_name: String,
    pub reason: String,
}

/// The backend's answer for a run. A backend that could not start returns no components and one diagnostic, never an
/// error: the structural result is always a complete answer on its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SemanticOutcome {
    pub components: Vec<SemanticComponent>,
    pub fallbacks: Vec<SemanticFallback>,
    pub diagnostics: Vec<Diagnostic>,
}

/// A component the checker may be asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticTarget {
    pub file_path: Utf8PathBuf,
    pub display_name: String,
    pub exported: bool,
    /// Props the structural pass already has a description for. The checker is not asked for those: it is 65% of
    /// the requests on Mantine, and the merge keeps the structural description when the checker's is empty.
    pub described: rustc_hash::FxHashSet<String>,
}

/// A checker that answers for components as they are asked about, for a long-lived session.
///
/// `request` never blocks: it queues the targets and returns the receiver that yields the outcome once the checker
/// has it, so a caller can wait a bounded time and take the structural answer when the checker is slower.
pub trait SemanticProvider: Send + Sync {
    fn request(&self, targets: Vec<SemanticTarget>) -> std::sync::mpsc::Receiver<SemanticOutcome>;
}

impl SemanticOutcome {
    /// An outcome for a backend that did nothing: the structural result stands, for this reason.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        let diagnostic = Diagnostic::new(
            DiagnosticSeverity::Warning,
            DiagnosticCode::SemanticBackend,
            format!("TypeScript checker backend unavailable: {}", reason.into()),
        )
        .with_help("The structural result is used instead. Install `typescript@beta` (7.1) or pass `--tsc-path`.");
        Self { diagnostics: vec![diagnostic], ..Self::default() }
    }
}

/// Replaces `entry`'s props with the checker's, keeping what only the structural pass has (a default value, tags,
/// `parent`, `declarations`, and a description when the checker has none). `shared` interns identical props across
/// the components of one merge.
pub fn apply_semantic_props(
    entry: &mut ComponentEntry,
    semantic: &SemanticComponent,
    shared: &mut FxHashMap<u64, Vec<Arc<ParsedProp>>>,
) {
    let mut props: BTreeMap<String, Arc<ParsedProp>> = BTreeMap::new();
    for prop in &semantic.props {
        let structural = entry.props.get(&prop.name);
        let description = if prop.description.is_empty() {
            structural.map(|s| s.description.clone()).unwrap_or_default()
        } else {
            prop.description.clone()
        };
        let merged = ParsedProp::new(
            prop.name.clone(),
            prop.prop_type.clone(),
            prop.required,
            structural.and_then(|s| s.default_value.clone()),
            description,
            structural.map(|s| s.tags.clone()).unwrap_or_default(),
            structural.and_then(|s| s.parent.clone()),
            structural.map(|s| s.declarations.clone()).unwrap_or_default(),
        );
        let mut hasher = FxHasher::default();
        merged.structural_hash(&mut hasher);
        let bucket = shared.entry(hasher.finish()).or_default();
        let merged = match bucket.iter().find(|existing| ***existing == merged) {
            Some(existing) => existing.clone(),
            None => {
                let merged = Arc::new(merged);
                bucket.push(merged.clone());
                merged
            }
        };
        props.insert(prop.name.clone(), merged);
    }
    entry.props = props;
}

/// Merge the checker's props over `output`, component by component.
///
/// Per `docs/type-checker-integration.md`: the checker decides which props exist, their types and whether they are
/// required; a description comes from the checker, else the structural pass; a default value stays structural (the
/// checker has no initializer for a destructured parameter); `parent` and `declarations` stay with the structural
/// prop of the same name. Inheritance layers and curated inherited props stay structural. Components the checker did
/// not answer for keep their structural result untouched.
///
/// Records provenance as `Info` diagnostics whose `affects` lists the output keys, so the canonical JSON gains no
/// field: one for the components the checker resolved, one per fallback reason.
pub fn merge_semantic(output: &mut ExtractionOutput, outcome: SemanticOutcome) {
    let SemanticOutcome { components, fallbacks, diagnostics } = outcome;
    let mut merged_keys = Vec::new();
    let mut shared: FxHashMap<u64, Vec<Arc<ParsedProp>>> = FxHashMap::default();
    for semantic in components {
        let Some((key, entry)) = output
            .components
            .iter_mut()
            .find(|(_, entry)| entry.file_path == semantic.file_path && entry.display_name == semantic.display_name)
        else {
            continue;
        };
        apply_semantic_props(entry, &semantic, &mut shared);
        merged_keys.push(key.clone());
    }
    output.diagnostics.extend(diagnostics);

    // A structural diagnostic about a prop the checker now answers for would be stale; the resolved component keeps
    // none of its degrade diagnostics.
    if !merged_keys.is_empty() {
        output.diagnostics.retain(|diagnostic| {
            let degrade = matches!(
                diagnostic.code,
                DiagnosticCode::OpaqueType | DiagnosticCode::MaxDepthExceeded | DiagnosticCode::CircularReference
            );
            !(degrade
                && !diagnostic.affects.is_empty()
                && diagnostic.affects.iter().all(|key| merged_keys.contains(key)))
        });
        merged_keys.sort();
        let mut provenance = Diagnostic::new(
            DiagnosticSeverity::Info,
            DiagnosticCode::SemanticBackend,
            format!("{} component(s) resolved by the TypeScript checker", merged_keys.len()),
        );
        provenance.affects = merged_keys;
        output.diagnostics.push(provenance);
    }

    let mut by_reason: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for fallback in fallbacks {
        let Some((key, _)) = output
            .components
            .iter()
            .find(|(_, entry)| entry.file_path == fallback.file_path && entry.display_name == fallback.display_name)
        else {
            continue;
        };
        by_reason.entry(fallback.reason).or_default().push(key.clone());
    }
    for (reason, mut keys) in by_reason {
        keys.sort();
        let mut diagnostic = Diagnostic::new(
            DiagnosticSeverity::Info,
            DiagnosticCode::SemanticBackend,
            format!("{} component(s) kept their structural result: {reason}", keys.len()),
        );
        diagnostic.affects = keys;
        output.diagnostics.push(diagnostic);
    }
}

#[cfg(test)]
mod tests;
