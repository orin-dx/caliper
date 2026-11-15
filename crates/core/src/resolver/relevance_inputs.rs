//! The lane-independent relevance evidence of a component (ADR 0028, slice T7), gathered with the resolver's own import
//! resolution: its props type's syntax, its body's reads and forwards, and its call sites across the project. Both
//! lanes decide from it ([`crate::relevance::annotate`]).

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::{FxHashMap, FxHashSet};

use super::ResolutionContext;
use super::import::{
    follow_reexports, lookup_interface, lookup_type_alias, resolve_import_specifier, resolve_to_canonical,
};
use crate::relevance::call_sites::{self, FileUsages};
use crate::relevance::forwarding::{self, Lane, Project};
use crate::relevance::props_type::{Declaration, PropsType};
use crate::relevance::{Body, CallSites, Inputs, Member};
use crate::types::ComponentMapping;
use crate::types::output::RelevanceEvidence;

/// A component's place in `global.component_mappings`: its file, name and span, which tell two mappings apart.
pub(crate) type SiteKey = (Utf8PathBuf, String, u32);

/// The call-site counts of every component of the project, by [`SiteKey`]; only components with evidence have one.
pub(crate) type ProjectSites = FxHashMap<SiteKey, CallSites>;

/// The relevance evidence of `mapping`.
pub(crate) fn evidence(mapping: &ComponentMapping, ctx: &ResolutionContext) -> RelevanceEvidence {
    let lane = Resolver { ctx };
    let props_type = lane.props_type(mapping);
    let project = Project { components: &ctx.global.component_mappings, imports: &ctx.import_map };
    let body = forwarding::forwarding(mapping, project, &lane).map(|forwarding| Body {
        reads: mapping.body.as_ref().map(|facts| facts.reads.clone()).unwrap_or_default(),
        defaulted: Vec::new(),
        elements: forwarding.elements,
        handed_on: forwarding.handed_on,
    });
    let key = (mapping.file_path.clone(), mapping.component_name.clone(), mapping.span_start);
    let call_sites = ctx.relevance_sites.get_or_init(|| project_sites(ctx)).get(&key).cloned().unwrap_or_default();
    RelevanceEvidence(Some(Arc::new(Inputs { props_type, body, call_sites })))
}

/// Counts every component's call sites once per context: the project's files' JSX and the components' bodies.
fn project_sites(ctx: &ResolutionContext) -> ProjectSites {
    let components = &ctx.global.component_mappings;
    let files: Vec<FileUsages<'_>> =
        ctx.global.jsx_usages.iter().map(|(file, usages)| FileUsages { file, usages }).collect();
    let canonical = |file: &Utf8Path, name: &str| resolve_to_canonical(name, file, ctx, &mut Vec::new());
    call_sites::count(components, &files, canonical)
        .into_iter()
        .zip(components)
        .filter(|(sites, _)| sites.sites > 0)
        .map(|(sites, mapping)| {
            ((mapping.file_path.clone(), mapping.component_name.clone(), mapping.span_start), sites)
        })
        .collect()
}

/// The resolver as the lane `forwarding` asks.
struct Resolver<'c> {
    ctx: &'c ResolutionContext,
}

impl Resolver<'_> {
    /// What `mapping`'s props type writes, each name it writes found where its file's imports declare it.
    fn props_type(&self, mapping: &ComponentMapping) -> PropsType {
        let ctx = self.ctx;
        PropsType::read(mapping, |name| {
            let (file, name) = resolve_to_canonical(name, &mapping.file_path, ctx, &mut Vec::new())
                .unwrap_or_else(|| (mapping.file_path.clone(), name.to_owned()));
            lookup_interface(ctx, file.as_str(), &name)
                .map(Declaration::Interface)
                .or_else(|| lookup_type_alias(ctx, file.as_str(), &name).map(|(_, alias)| Declaration::Alias(alias)))
        })
    }
}

impl Lane for Resolver<'_> {
    fn declaration(&self, from: &Utf8Path, specifier: &str, export: &str) -> Option<(Utf8PathBuf, String)> {
        let mut diagnostics = Vec::new();
        let file = resolve_import_specifier(specifier, from, self.ctx, &mut diagnostics)?;
        Some(follow_reexports(file, export.to_owned(), self.ctx, &mut diagnostics, &mut FxHashSet::default(), 0))
    }

    /// The target's own props from its props type's syntax: the same in both lanes, and known before the target is
    /// resolved.
    fn members(&self, component: &ComponentMapping) -> Option<Vec<Member>> {
        Some(self.props_type(component).own_members())
    }
}
