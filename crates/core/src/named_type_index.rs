//! Named-type resolution index — interfaces, type aliases, and their declared
//! generic type parameters, indexed for fast `(file, name)` lookup.
//!
//! Built once from `GlobalSourceData` before parallel resolution starts,
//! mirroring `import_map.rs::ImportResolutionMap`'s split: `GlobalSourceData`'s
//! own maps stay flat (`FxHashMap<String, T>`, unchanged — `merge()` and the
//! resolver test module's fixture-building code both depend on that shape),
//! while this index restructures them into nested
//! `FxHashMap<Utf8PathBuf, FxHashMap<CompactString, T>>` maps so a lookup can
//! borrow `&Utf8Path`/`&str` directly instead of allocating a fresh
//! `"{file}:{name}"` String per call. `lookup_interface`/`lookup_type_alias`
//! (the two hottest of these) run recursively for every prop, every generic
//! argument, and every extends clause across the whole component tree — the
//! same cost `ImportResolutionMap` and `ResolutionContext::enum_bare_index`
//! were already built to eliminate for import/enum lookups elsewhere in this
//! resolver.

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::path_key::PathKey;

use crate::types::{
    CollectedInterface, CollectedType, CollectedTypeAlias, EnumValue, GlobalSourceData, SourceData, TypeName,
};

/// The project's declarations. Shared behind an `Arc` so a lazy session's context rebuild, which sets a new
/// dependency `fallback`, copies a pointer and not every interface and alias of the project.
#[derive(Clone, Default)]
pub(crate) struct IndexMaps {
    interfaces: FxHashMap<PathKey, FxHashMap<CompactString, CollectedInterface>>,
    type_aliases: FxHashMap<PathKey, FxHashMap<CompactString, CollectedTypeAlias>>,
    type_alias_params: FxHashMap<PathKey, FxHashMap<CompactString, Vec<TypeName>>>,
    interface_type_params: FxHashMap<PathKey, FxHashMap<CompactString, Vec<TypeName>>>,
    /// Type parameter defaults of the aliases and interfaces above (`SourceData::type_param_defaults`).
    type_param_defaults: FxHashMap<PathKey, FxHashMap<CompactString, Vec<Option<CollectedType>>>>,
    /// The array-typed consts each file declares, by their literal members (`SourceData::const_arrays`): what
    /// `(typeof X)[number]` reads as the symbol `X` in scope in the file that wrote the type.
    const_arrays: FxHashMap<PathKey, FxHashMap<CompactString, Vec<EnumValue>>>,
    /// Files declaring `React.`-qualified names (`@types/react`), the only ones a qualified fallback can match.
    react_namespaced: FxHashSet<PathKey>,
}

#[derive(Clone, Default)]
pub(crate) struct NamedTypeIndex {
    maps: std::sync::Arc<IndexMaps>,
    /// Loaded dependency declarations (ADR 0010), consulted for a file this index holds nothing for. Shared by every
    /// watch revision, never copied.
    pub(crate) fallback: Option<std::sync::Arc<NamedTypeIndex>>,
}

impl std::ops::Deref for NamedTypeIndex {
    type Target = IndexMaps;

    fn deref(&self) -> &IndexMaps {
        &self.maps
    }
}

/// What `lookup_ambient` found at a given ambient lib file, mirroring the
/// two shapes a same-file/imported lookup can also produce.
pub(crate) enum AmbientMatch<'a> {
    Interface,
    TypeAlias(&'a CollectedTypeAlias),
}

impl NamedTypeIndex {
    /// Build from a fully-merged `GlobalSourceData`. Called once per
    /// resolution pass (`ResolutionContext::build`), before parallel
    /// resolution starts. No I/O — pure data transformation.
    ///
    /// Splits each flat `"{file}:{name}"` key on its *last* `:` — the same
    /// technique `ResolutionContext::enum_bare_index`/`const_array_bare_index`
    /// already use — rather than requiring a value-embedded `file_path` to
    /// anchor the split, so this works uniformly across all four maps
    /// (`type_alias_params`/`interface_type_params`'s own values carry no
    /// `file_path` of their own to anchor on).
    pub(crate) fn build(global: &GlobalSourceData) -> Self {
        let mut index = Self::default();
        index.insert_flat_maps(
            &global.interfaces,
            &global.interface_type_params,
            &global.type_aliases,
            &global.type_alias_params,
            &global.type_param_defaults,
        );
        index.insert_const_arrays(&global.const_arrays);
        index
    }

    /// Remove `file`'s entries from every map — mirrors
    /// `GlobalSourceData::remove_file`'s per-file semantics, so
    /// `WatchSession::update_file` can keep this index in sync incrementally
    /// instead of rebuilding the whole project's index (every interface and
    /// type alias, cloned again) on every single-file edit. Confirmed as a
    /// real, measured regression via `crates/core/benches/extraction.rs`'s
    /// `incremental_update` group before this existed — rebuilding via `build`
    /// there roughly doubled per-edit latency, since a full-project index
    /// rebuild wasn't amortized the way it is by `extract()`'s one-shot path
    /// (which resolves every component in one run, not just the few affected
    /// by a single edit).
    pub(crate) fn remove_file(&mut self, file: &Utf8Path) {
        let maps = std::sync::Arc::make_mut(&mut self.maps);
        maps.interfaces.remove(file.as_str());
        maps.type_aliases.remove(file.as_str());
        maps.type_alias_params.remove(file.as_str());
        maps.interface_type_params.remove(file.as_str());
        maps.type_param_defaults.remove(file.as_str());
        maps.const_arrays.remove(file.as_str());
        maps.react_namespaced.remove(file.as_str());
    }

    /// Insert one freshly re-parsed file's own (not yet merged) interfaces/
    /// type aliases into this index — the incremental counterpart to `build`'s
    /// whole-project walk. Callers should `remove_file` first so a renamed or
    /// removed declaration doesn't linger under its old name.
    pub(crate) fn merge_file(&mut self, data: &SourceData) {
        self.insert_flat_maps(
            &data.interfaces,
            &data.interface_type_params,
            &data.type_aliases,
            &data.type_alias_params,
            &data.type_param_defaults,
        );
        self.insert_const_arrays(&data.const_arrays);
    }

    /// Splits each flat `"{file}:{name}"` key on its *last* `:` — the same
    /// technique `ResolutionContext::enum_bare_index`/`const_array_bare_index`
    /// already use — rather than requiring a value-embedded `file_path` to
    /// anchor the split, so this works uniformly across all four maps
    /// (`type_alias_params`/`interface_type_params`'s own values carry no
    /// `file_path` of their own to anchor on). Shared by `build` (over a
    /// fully-merged `GlobalSourceData`) and `merge_file` (over one file's own
    /// `SourceData`) — both have the identical flat-map shape.
    fn insert_flat_maps(
        &mut self,
        interfaces: &FxHashMap<std::string::String, CollectedInterface>,
        interface_type_params: &FxHashMap<std::string::String, Vec<TypeName>>,
        type_aliases: &FxHashMap<std::string::String, CollectedTypeAlias>,
        type_alias_params: &FxHashMap<std::string::String, Vec<TypeName>>,
        type_param_defaults: &FxHashMap<std::string::String, Vec<Option<CollectedType>>>,
    ) {
        let maps = std::sync::Arc::make_mut(&mut self.maps);
        for (key, defaults) in type_param_defaults {
            let Some((file, name)) = key.rsplit_once(':') else { continue };
            let file = PathKey::from(Utf8Path::new(file));
            maps.type_param_defaults.entry(file).or_default().insert(CompactString::from(name), defaults.clone());
        }
        for (key, iface) in interfaces {
            let Some((file, name)) = key.rsplit_once(':') else { continue };
            let file = PathKey::from(Utf8Path::new(file));
            if name.starts_with("React.") {
                maps.react_namespaced.insert(file.clone());
            }
            maps.interfaces.entry(file.clone()).or_default().insert(CompactString::from(name), iface.clone());
            if let Some(params) = interface_type_params.get(key) {
                maps.interface_type_params.entry(file).or_default().insert(CompactString::from(name), params.clone());
            }
        }

        for (key, alias) in type_aliases {
            let Some((file, name)) = key.rsplit_once(':') else { continue };
            let file = PathKey::from(Utf8Path::new(file));
            if name.starts_with("React.") {
                maps.react_namespaced.insert(file.clone());
            }
            maps.type_aliases.entry(file.clone()).or_default().insert(CompactString::from(name), alias.clone());
            if let Some(params) = type_alias_params.get(key) {
                maps.type_alias_params.entry(file).or_default().insert(CompactString::from(name), params.clone());
            }
        }
    }

    fn insert_const_arrays(&mut self, const_arrays: &FxHashMap<std::string::String, Vec<EnumValue>>) {
        let maps = std::sync::Arc::make_mut(&mut self.maps);
        for (key, values) in const_arrays {
            let Some((file, name)) = key.rsplit_once(':') else { continue };
            let file = PathKey::from(Utf8Path::new(file));
            maps.const_arrays.entry(file).or_default().insert(CompactString::from(name), values.clone());
        }
    }

    /// This index with `fallback` as its dependency layer. The project's declarations are shared, not copied.
    pub(crate) fn with_fallback(&self, fallback: std::sync::Arc<NamedTypeIndex>) -> NamedTypeIndex {
        NamedTypeIndex { maps: self.maps.clone(), fallback: Some(fallback) }
    }

    /// Whether `other` shares this index's project declarations, not a copy of them (tests).
    #[cfg(test)]
    pub(crate) fn shares_declarations_with(&self, other: &NamedTypeIndex) -> bool {
        std::sync::Arc::ptr_eq(&self.maps, &other.maps)
    }

    /// The index holding `file`'s declarations: this one, or the dependency fallback for a file this one doesn't hold.
    /// Project and dependency files never overlap, so a file is in exactly one of them.
    fn layer_for(&self, file: &Utf8Path) -> &NamedTypeIndex {
        match &self.fallback {
            Some(fallback)
                if !self.interfaces.contains_key(file.as_str()) && !self.type_aliases.contains_key(file.as_str()) =>
            {
                fallback
            }
            _ => self,
        }
    }

    /// Look up an interface by canonical `(file, name)`, falling back to a
    /// `React.`-namespace-qualified name. `@types/react` declares everything
    /// inside `declare namespace React { ... }`, so real declarations are
    /// keyed "React.HTMLAttributes" even though nothing imports them that
    /// way — a plain `import type { HTMLAttributes } from "react"` binds the
    /// bare name.
    pub(crate) fn lookup_interface(&self, file: &Utf8Path, name: &str) -> Option<&CollectedInterface> {
        let index = self.layer_for(file);
        let per_file = index.interfaces.get(file.as_str())?;
        per_file.get(name).or_else(|| {
            index.react_namespaced.contains(file.as_str()).then(|| per_file.get(format!("React.{name}").as_str()))?
        })
    }

    /// Same as `lookup_interface`, for type aliases. Returns the matched name
    /// (bare or `React.`-qualified) alongside the value, since callers also
    /// need it to look up `type_alias_params` under the same `(file, name)`.
    pub(crate) fn lookup_type_alias(
        &self,
        file: &Utf8Path,
        name: &str,
    ) -> Option<(CompactString, &CollectedTypeAlias)> {
        let index = self.layer_for(file);
        let per_file = index.type_aliases.get(file.as_str())?;
        if let Some(alias) = per_file.get(name) {
            return Some((CompactString::from(name), alias));
        }
        if !index.react_namespaced.contains(file.as_str()) {
            return None;
        }
        let qualified = format!("React.{name}");
        per_file.get(qualified.as_str()).map(|alias| (CompactString::from(qualified), alias))
    }

    /// The members of the array-typed const `name` declared at `file`, in this index or the dependency fallback.
    pub(crate) fn lookup_const_array(&self, file: &Utf8Path, name: &str) -> Option<&[EnumValue]> {
        self.const_arrays
            .get(file.as_str())
            .and_then(|per_file| per_file.get(name))
            .map(Vec::as_slice)
            .or_else(|| self.fallback.as_ref()?.lookup_const_array(file, name))
    }

    /// Whether `name` is actually declared at `file` — mirrors the bare/
    /// `React.`-qualified key fallback `lookup_interface`/`lookup_type_alias`
    /// use, so a barrel-chain hop is considered "found" by the same rule
    /// those lookups do.
    pub(crate) fn is_declared_at(&self, file: &Utf8Path, name: &str) -> bool {
        self.lookup_interface(file, name).is_some() || self.lookup_type_alias(file, name).is_some()
    }

    /// The declared defaults of `name`'s type parameters at `file`, when any has one. `name` is the exact key, as
    /// `lookup_type_alias` returns it or an interface's `scoped_key` ends.
    pub(crate) fn lookup_type_param_defaults(&self, file: &Utf8Path, name: &str) -> Option<&[Option<CollectedType>]> {
        self.layer_for(file).type_param_defaults.get(file.as_str())?.get(name).map(Vec::as_slice)
    }

    /// `lookup_type_param_defaults` for an interface the caller already has in hand.
    pub(crate) fn lookup_interface_param_defaults_for(
        &self,
        iface: &CollectedInterface,
    ) -> Option<&[Option<CollectedType>]> {
        let name = iface.scoped_key.rsplit_once(':').map(|(_, name)| name).unwrap_or(&iface.scoped_key);
        self.lookup_type_param_defaults(&iface.file_path, name)
    }

    pub(crate) fn lookup_type_alias_params(&self, file: &Utf8Path, name: &str) -> Option<&Vec<TypeName>> {
        self.layer_for(file).type_alias_params.get(file.as_str())?.get(name)
    }

    /// Declared generic type parameters for an interface the caller already
    /// has in hand (its own declared `interface Foo<TData>`, not a reference
    /// to it). Derives the lookup key from `iface.scoped_key` via the same
    /// zero-allocation `rsplit_once(':')` split `ResolutionContext`'s
    /// `enum_bare_index`/`const_array_bare_index` already use, rather than
    /// requiring the caller to have a separate bare name on hand.
    pub(crate) fn lookup_interface_type_params_for(&self, iface: &CollectedInterface) -> Option<&Vec<TypeName>> {
        let name = iface.scoped_key.rsplit_once(':').map(|(_, name)| name).unwrap_or(&iface.scoped_key);
        self.lookup_interface_type_params(&iface.file_path, name)
    }

    fn lookup_interface_type_params(&self, file: &Utf8Path, name: &str) -> Option<&Vec<TypeName>> {
        self.layer_for(file).interface_type_params.get(file.as_str())?.get(name)
    }

    /// Same as `lookup_interface`, but also falls back to TypeScript's own
    /// ambient lib files when the name isn't found via import/same-file
    /// resolution — e.g. `HTMLDivElement`, declared ambiently in
    /// `lib.dom.d.ts`, is never imported.
    pub(crate) fn lookup_interface_including_ambient(
        &self,
        file: &Utf8Path,
        name: &str,
        ambient_global_files: &[Utf8PathBuf],
    ) -> Option<&CollectedInterface> {
        self.lookup_interface(file, name).or_else(|| {
            ambient_global_files.iter().find_map(|lib_file| self.interfaces.get(lib_file.as_str())?.get(name))
        })
    }

    /// Look up a name directly on TypeScript's own ambient lib files, by bare
    /// name only (these are ambient globals, never namespace-qualified).
    /// Deliberately separate from — and only ever consulted after — every
    /// other resolution path; see the original call site's doc comment for why.
    pub(crate) fn lookup_ambient(&self, ambient_global_files: &[Utf8PathBuf], name: &str) -> Option<AmbientMatch<'_>> {
        for lib_file in ambient_global_files {
            if self.interfaces.get(lib_file.as_str()).and_then(|m| m.get(name)).is_some() {
                return Some(AmbientMatch::Interface);
            }
            if let Some(alias) = self.type_aliases.get(lib_file.as_str()).and_then(|m| m.get(name)) {
                return Some(AmbientMatch::TypeAlias(alias));
            }
        }
        None
    }

    /// A type alias of TypeScript's own lib files by bare name, with the lib file declaring it (for its parameters).
    pub(crate) fn lookup_ambient_alias<'a>(
        &'a self,
        ambient_global_files: &'a [Utf8PathBuf],
        name: &str,
    ) -> Option<(&'a Utf8Path, &'a CollectedTypeAlias)> {
        ambient_global_files.iter().find_map(|lib_file| {
            let alias = self.type_aliases.get(lib_file.as_str())?.get(name)?;
            Some((lib_file.as_path(), alias))
        })
    }

    /// Look up an interface directly by canonical `(file, name)` — no
    /// bare/`React.`-qualified fallback. Used where the caller already has
    /// an exact, known-good key (e.g. `real_html_attrs_chain`'s merged
    /// `@types/react` lookup, which tries both forms itself in a specific
    /// priority order).
    pub(crate) fn lookup_interface_exact(&self, file: &Utf8Path, name: &str) -> Option<&CollectedInterface> {
        self.layer_for(file).interfaces.get(file.as_str())?.get(name)
    }

    /// Same as `lookup_interface_exact`, for type aliases — no bare/`React.`-
    /// qualified fallback. `type_alias_params` and `type_aliases` are always
    /// populated under the identical (file, name) key by the extractor, so a
    /// caller that already confirmed `lookup_type_alias_params` succeeded on
    /// an exact key can use this to fetch the matching alias without
    /// re-deriving a fallback that can't actually diverge.
    pub(crate) fn lookup_type_alias_exact(&self, file: &Utf8Path, name: &str) -> Option<&CollectedTypeAlias> {
        self.layer_for(file).type_aliases.get(file.as_str())?.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TypeName;
    use camino::Utf8PathBuf;

    fn make_interface(scoped_key: &str, file_path: &str, name: &str) -> CollectedInterface {
        CollectedInterface {
            scoped_key: scoped_key.to_owned(),
            name: TypeName::from(name),
            file_path: Utf8PathBuf::from(file_path),
            props: vec![],
            extends: vec![],
            description: String::new(),
            tags: Default::default(),
        }
    }

    #[test]
    fn lookup_interface_finds_a_bare_key() {
        let mut global = GlobalSourceData::default();
        let key = "/project/src/Button.tsx:ButtonProps".to_owned();
        global.interfaces.insert(key.clone(), make_interface(&key, "/project/src/Button.tsx", "ButtonProps"));

        let index = NamedTypeIndex::build(&global);
        let file = Utf8PathBuf::from("/project/src/Button.tsx");
        let found = index.lookup_interface(&file, "ButtonProps");
        assert!(found.is_some(), "expected to find ButtonProps");
        assert_eq!(found.unwrap().name.as_str(), "ButtonProps");
    }

    #[test]
    fn lookup_interface_falls_back_to_react_qualified_key() {
        let mut global = GlobalSourceData::default();
        let key = "/lib/react.d.ts:React.HTMLAttributes".to_owned();
        global.interfaces.insert(key.clone(), make_interface(&key, "/lib/react.d.ts", "HTMLAttributes"));

        let index = NamedTypeIndex::build(&global);
        let file = Utf8PathBuf::from("/lib/react.d.ts");
        // Looked up by the bare name, exactly as a plain `import type { HTMLAttributes }` would.
        let found = index.lookup_interface(&file, "HTMLAttributes");
        assert!(found.is_some(), "expected the React.-qualified fallback to match");
    }

    #[test]
    fn lookup_interface_returns_none_for_a_different_file() {
        let mut global = GlobalSourceData::default();
        let key = "/project/src/Button.tsx:ButtonProps".to_owned();
        global.interfaces.insert(key.clone(), make_interface(&key, "/project/src/Button.tsx", "ButtonProps"));

        let index = NamedTypeIndex::build(&global);
        let other_file = Utf8PathBuf::from("/project/src/Other.tsx");
        assert!(index.lookup_interface(&other_file, "ButtonProps").is_none());
    }

    #[test]
    fn is_declared_at_is_true_for_an_interface_and_false_for_an_unrelated_name() {
        let mut global = GlobalSourceData::default();
        let key = "/project/src/Button.tsx:ButtonProps".to_owned();
        global.interfaces.insert(key.clone(), make_interface(&key, "/project/src/Button.tsx", "ButtonProps"));

        let index = NamedTypeIndex::build(&global);
        let file = Utf8PathBuf::from("/project/src/Button.tsx");
        assert!(index.is_declared_at(&file, "ButtonProps"));
        assert!(!index.is_declared_at(&file, "SomethingElse"));
    }

    #[test]
    fn interface_type_params_are_indexed_under_the_same_file_and_name_as_the_interface() {
        let mut global = GlobalSourceData::default();
        let key = "/project/src/List.tsx:ListProps".to_owned();
        global.interfaces.insert(key.clone(), make_interface(&key, "/project/src/List.tsx", "ListProps"));
        global.interface_type_params.insert(key, vec![TypeName::from("TItem")]);

        let index = NamedTypeIndex::build(&global);
        let file = Utf8PathBuf::from("/project/src/List.tsx");
        let params = index.lookup_interface_type_params(&file, "ListProps");
        assert!(params.is_some(), "expected declared type params to be indexed");
        assert_eq!(params.unwrap(), &vec![TypeName::from("TItem")]);
    }

    #[test]
    fn lookup_ambient_checks_ambient_files_in_order_and_distinguishes_interface_from_alias() {
        let mut global = GlobalSourceData::default();
        let dom_key = "/lib/lib.dom.d.ts:HTMLDivElement".to_owned();
        global.interfaces.insert(dom_key.clone(), make_interface(&dom_key, "/lib/lib.dom.d.ts", "HTMLDivElement"));

        let index = NamedTypeIndex::build(&global);
        let ambient_files = vec![Utf8PathBuf::from("/lib/lib.es5.d.ts"), Utf8PathBuf::from("/lib/lib.dom.d.ts")];
        match index.lookup_ambient(&ambient_files, "HTMLDivElement") {
            Some(AmbientMatch::Interface) => {}
            other => panic!("expected AmbientMatch::Interface, got a different result: {}", other.is_some()),
        }
        assert!(index.lookup_ambient(&ambient_files, "NeverDeclared").is_none());
    }
}
