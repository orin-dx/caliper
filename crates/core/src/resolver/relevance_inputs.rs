//! The relevance evidence of a component (ADR 0028, slice T7), gathered with the resolver's own import resolution. What
//! its props type writes and what its body does are read while its resolution records dependencies
//! (`resolve_component_tracked`), so an edit to a file they read refreshes them. Its call sites across the project are
//! counted once per resolution context ([`project_sites`]) and joined where the answer becomes output, never kept with
//! a resolution: an edit to any consumer changes them.

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::{FxHashMap, FxHashSet};

use super::ResolutionContext;
use super::import::{
    follow_reexports, lookup_interface, lookup_type_alias, resolve_import_specifier, resolve_to_canonical,
};
use crate::relevance::call_sites::{self, FileUsages, SiteKey, site_key};
use crate::relevance::forwarding::{self, Lane, Project};
use crate::relevance::props_type::{Declaration, PropsType};
use crate::relevance::{Body, CallSites, Inputs, Member};
use crate::types::ComponentMapping;

/// The call-site counts of every component of the project, by [`SiteKey`]; only components with evidence have one.
pub(crate) type ProjectSites = FxHashMap<SiteKey, CallSites>;

/// The relevance evidence of `mapping` its resolution reads: its props type and its body.
pub(crate) fn evidence(mapping: &ComponentMapping, ctx: &ResolutionContext) -> Arc<Inputs> {
    let lane = Resolver { ctx };
    let props_type = lane.props_type(mapping);
    let project = Project { components: &ctx.global.component_mappings, imports: &ctx.import_map };
    let body = forwarding::forwarding(mapping, project, &lane).map(|forwarding| Body {
        reads: mapping.body.as_ref().map(|facts| facts.reads.clone()).unwrap_or_default(),
        elements: forwarding.elements,
        handed_on: forwarding.handed_on,
    });
    let narrows = ctx.collapse_inherited_props != crate::types::options::CollapseInheritedProps::None;
    Arc::new(Inputs { site: site_key(mapping), props_type, body, narrows })
}

/// Every component's call sites in `ctx`'s project (the files' JSX and the components' bodies), counted once per
/// context.
pub(crate) fn project_sites(ctx: &ResolutionContext) -> Arc<ProjectSites> {
    ctx.relevance_sites.get_or_init(|| Arc::new(count(ctx))).clone()
}

fn count(ctx: &ResolutionContext) -> ProjectSites {
    let components = &ctx.global.component_mappings;
    let files: Vec<FileUsages<'_>> =
        ctx.global.jsx_usages.iter().map(|(file, usages)| FileUsages { file, usages }).collect();
    let canonical = |file: &Utf8Path, name: &str| resolve_to_canonical(name, file, ctx, &mut Vec::new());
    call_sites::count(components, &files, canonical)
        .into_iter()
        .zip(components)
        .filter(|(sites, _)| sites.sites > 0)
        .map(|(sites, mapping)| (site_key(mapping), sites))
        .collect()
}

/// The resolver as the lane `forwarding` asks.
struct Resolver<'c> {
    ctx: &'c ResolutionContext,
}

impl Resolver<'_> {
    /// What `mapping`'s props type writes, each name it writes found where the file writing it imports or declares it.
    fn props_type(&self, mapping: &ComponentMapping) -> PropsType {
        let ctx = self.ctx;
        PropsType::read(mapping, |name: &str, from: &Utf8Path| {
            let (file, name) = resolve_to_canonical(name, from, ctx, &mut Vec::new())
                .unwrap_or_else(|| (from.to_owned(), name.to_owned()));
            lookup_interface(ctx, file.as_str(), &name)
                .map(Declaration::Interface)
                .or_else(|| lookup_type_alias(ctx, file.as_str(), &name).map(|(_, alias)| Declaration::Alias(alias)))
                .map(|declaration| (declaration, file))
        })
    }
}

impl Lane for Resolver<'_> {
    fn declaration(&self, from: &Utf8Path, specifier: &str, export: &str) -> Option<(Utf8PathBuf, String)> {
        let mut diagnostics = Vec::new();
        let file = resolve_import_specifier(specifier, from, self.ctx, &mut diagnostics)?;
        Some(follow_reexports(file, export.to_owned(), self.ctx, &mut diagnostics, &mut FxHashSet::default(), 0))
    }

    /// The target's props outside the ambient declarations, its bases' included, from its props type's syntax: the
    /// same in both lanes, and known before the target is resolved.
    fn members(&self, component: &ComponentMapping) -> Option<Vec<Member>> {
        Some(self.props_type(component).reachable_members())
    }
}
