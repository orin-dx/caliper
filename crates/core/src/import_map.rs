//! Import resolution map — built from `GlobalSourceData`, used read-only during resolution.
//!
//! Answers the question: "in file F, the name N was imported — what specifier and
//! canonical name does it refer to?"  Actual path resolution (specifier → absolute
//! path) is left to the caller via `oxc_resolver`.

use camino::Utf8Path;
use compact_str::CompactString;
use rustc_hash::FxHashMap;

use crate::path_key::PathKey;
use crate::types::{GlobalSourceData, LexedExport};

// ─── Public Types ─────────────────────────────────────────────────────────────

/// Where an imported name comes from, as written in the source.
#[derive(Debug, Clone)]
pub struct ImportRef {
    /// The import specifier as written: `"@radix-ui/react-button"`, `"./types"`, etc.
    pub specifier: String,
    /// The exported name in the source module (original name before `as` rename).
    pub exported_name: CompactString,
    /// `true` for `import type { ... }` bindings.
    #[allow(dead_code)]
    pub is_type_only: bool,
}

/// Result of walking one step of a re-export chain.
#[derive(Debug, Clone)]
pub enum ReExportStep {
    /// Found a named re-export — caller resolves `source_specifier` to a file and recurses.
    Named { source_specifier: String, source_name: String },
    /// Found one or more `export * from "..."` entries — caller checks each.
    Wildcards(Vec<String>),
}

// ─── ImportResolutionMap ──────────────────────────────────────────────────────

/// Maps `(consuming_file, local_name)` → import reference.
/// Built once from `GlobalSourceData`, then used read-only during parallel resolution.
pub struct ImportResolutionMap {
    /// `file_path` → (`local_name` → specifier + original exported name).
    /// Nested (rather than a flat `(Utf8PathBuf, CompactString)` key) so lookups can
    /// borrow `&Utf8Path` directly instead of cloning the whole path per call.
    /// Note: we store the specifier string, not the resolved file path.
    /// The resolver calls `oxc_resolver` to turn specifiers into absolute paths.
    bindings: FxHashMap<PathKey, FxHashMap<CompactString, ImportRef>>,
    /// `global.re_export_map`, keyed for string-hashed lookups during barrel walks.
    re_exports: FxHashMap<PathKey, Vec<LexedExport>>,
}

impl ImportResolutionMap {
    /// Build from a fully-merged `GlobalSourceData`.
    /// Called once per extraction run, before parallel resolution starts.
    ///
    /// No I/O — pure data transformation.
    pub fn build(global: &GlobalSourceData) -> Self {
        let mut bindings: FxHashMap<PathKey, FxHashMap<CompactString, ImportRef>> = FxHashMap::default();

        for (file_path, import_bindings) in &global.import_map {
            let per_file = bindings.entry(PathKey::from(file_path.as_path())).or_default();
            for binding in import_bindings {
                per_file.insert(
                    binding.local_name.clone(),
                    ImportRef {
                        specifier: binding.specifier.clone(),
                        exported_name: binding.exported_name.clone(),
                        is_type_only: binding.is_type_only,
                    },
                );
            }
        }

        let re_exports = global
            .re_export_map
            .iter()
            .map(|(file_path, exports)| (PathKey::from(file_path.as_path()), exports.clone()))
            .collect();
        Self { bindings, re_exports }
    }

    /// Given a file and a local name used in that file, return where it was imported from.
    ///
    /// Returns `None` if the name is locally defined (not imported).
    pub fn find_import(&self, file: &Utf8Path, local_name: &str) -> Option<&ImportRef> {
        self.bindings.get(file.as_str())?.get(local_name)
    }

    /// Walk one step of the re-export chain for `barrel_file`.
    ///
    /// - If a `ReExportNamed` entry matches `exported_name`, returns `Named { .. }`.
    /// - If no named match is found but there are `ReExportAll` entries, returns `Wildcards(..)`.
    /// - Returns `None` if the barrel file has no relevant re-exports.
    pub fn resolve_reexport_chain(&self, barrel_file: &Utf8Path, exported_name: &str) -> Option<ReExportStep> {
        let exports = self.re_exports.get(barrel_file.as_str())?;

        let mut wildcard_specifiers: Vec<String> = Vec::new();

        for export in exports {
            match export {
                LexedExport::ReExportNamed { local_name, source_name, source_specifier, .. } => {
                    if local_name == exported_name {
                        return Some(ReExportStep::Named {
                            source_specifier: source_specifier.clone(),
                            source_name: source_name.clone(),
                        });
                    }
                }
                LexedExport::ReExportAll { source_specifier, .. } => {
                    wildcard_specifiers.push(source_specifier.clone());
                }
                // Namespace re-exports and local declarations are not relevant here.
                _ => {}
            }
        }

        if !wildcard_specifiers.is_empty() {
            Some(ReExportStep::Wildcards(wildcard_specifiers))
        } else {
            None
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GlobalSourceData, ImportBinding, LexedExport};
    use camino::Utf8PathBuf;
    use compact_str::CompactString;

    /// Convenience builder for `GlobalSourceData`.
    fn make_global(
        imports: Vec<(Utf8PathBuf, Vec<ImportBinding>)>,
        exports: Vec<(Utf8PathBuf, Vec<LexedExport>)>,
    ) -> GlobalSourceData {
        let mut g = GlobalSourceData::default();
        for (path, bindings) in imports {
            g.import_map.insert(path, bindings);
        }
        for (path, exps) in exports {
            g.re_export_map.insert(path, exps);
        }
        g
    }

    fn make_binding(local_name: &str, exported_name: &str, specifier: &str, is_type_only: bool) -> ImportBinding {
        ImportBinding {
            local_name: CompactString::from(local_name),
            exported_name: CompactString::from(exported_name),
            specifier: specifier.to_owned(),
            is_type_only,
        }
    }

    #[test]
    fn test_named_import_lookup() {
        let file = Utf8PathBuf::from("/project/src/Button.tsx");
        let global = make_global(
            vec![(file.clone(), vec![make_binding("ButtonProps", "ButtonProps", "@radix-ui/react-button", false)])],
            vec![],
        );

        let map = ImportResolutionMap::build(&global);

        let result = map.find_import(&file, "ButtonProps").expect("should find import");
        assert_eq!(result.specifier, "@radix-ui/react-button");
        assert_eq!(result.exported_name.as_str(), "ButtonProps");
        assert!(!result.is_type_only);
    }

    #[test]
    fn test_named_import_not_found() {
        let file = Utf8PathBuf::from("/project/src/Button.tsx");
        let global = make_global(vec![], vec![]);
        let map = ImportResolutionMap::build(&global);

        assert!(map.find_import(&file, "LocalType").is_none());
    }

    #[test]
    fn test_renamed_import() {
        // import { Button as Btn } from "@radix-ui/react-button"
        let file = Utf8PathBuf::from("/project/src/Wrapper.tsx");
        let global = make_global(
            vec![(file.clone(), vec![make_binding("Btn", "Button", "@radix-ui/react-button", false)])],
            vec![],
        );

        let map = ImportResolutionMap::build(&global);

        // Looking up by local alias "Btn"
        let result = map.find_import(&file, "Btn").expect("should find renamed import");
        assert_eq!(result.exported_name.as_str(), "Button");
        assert_eq!(result.specifier, "@radix-ui/react-button");

        // Original name "Button" is NOT a local binding in this file
        assert!(map.find_import(&file, "Button").is_none());
    }

    #[test]
    fn test_reexport_named_chain() {
        // barrel/index.ts: export { Foo as Bar } from "./foo"
        // Looking up "Bar" should return Named { source_name: "Foo", source_specifier: "./foo" }
        let barrel = Utf8PathBuf::from("/project/src/index.ts");
        let global = make_global(
            vec![],
            vec![(
                barrel.clone(),
                vec![LexedExport::ReExportNamed {
                    local_name: "Bar".to_owned(),
                    source_name: "Foo".to_owned(),
                    source_specifier: "./foo".to_owned(),
                    is_type_only: false,
                }],
            )],
        );

        let map = ImportResolutionMap::build(&global);

        match map.resolve_reexport_chain(&barrel, "Bar") {
            Some(ReExportStep::Named { source_specifier, source_name }) => {
                assert_eq!(source_specifier, "./foo");
                assert_eq!(source_name, "Foo");
            }
            other => panic!("expected Named, got {:?}", other),
        }
    }

    #[test]
    fn test_reexport_named_no_match() {
        // The barrel exports "Bar" but we ask for "Baz"
        let barrel = Utf8PathBuf::from("/project/src/index.ts");
        let global = make_global(
            vec![],
            vec![(
                barrel.clone(),
                vec![LexedExport::ReExportNamed {
                    local_name: "Bar".to_owned(),
                    source_name: "Foo".to_owned(),
                    source_specifier: "./foo".to_owned(),
                    is_type_only: false,
                }],
            )],
        );

        let map = ImportResolutionMap::build(&global);
        assert!(map.resolve_reexport_chain(&barrel, "Baz").is_none());
    }

    #[test]
    fn test_reexport_all() {
        // barrel/index.ts: export * from "./types"
        let barrel = Utf8PathBuf::from("/project/src/index.ts");
        let global = make_global(
            vec![],
            vec![(
                barrel.clone(),
                vec![LexedExport::ReExportAll { source_specifier: "./types".to_owned(), is_type_only: false }],
            )],
        );

        let map = ImportResolutionMap::build(&global);

        // resolve_reexport_chain for any name should return Wildcards
        match map.resolve_reexport_chain(&barrel, "AnyName") {
            Some(ReExportStep::Wildcards(specs)) => {
                assert_eq!(specs, vec!["./types"]);
            }
            other => panic!("expected Wildcards, got {:?}", other),
        }
    }

    #[test]
    fn test_reexport_named_takes_priority_over_wildcard() {
        // If a barrel has both a named re-export matching the name AND wildcards,
        // the named re-export wins (returned first).
        let barrel = Utf8PathBuf::from("/project/src/index.ts");
        let global = make_global(
            vec![],
            vec![(
                barrel.clone(),
                vec![
                    LexedExport::ReExportAll { source_specifier: "./everything".to_owned(), is_type_only: false },
                    LexedExport::ReExportNamed {
                        local_name: "Target".to_owned(),
                        source_name: "Target".to_owned(),
                        source_specifier: "./target".to_owned(),
                        is_type_only: false,
                    },
                ],
            )],
        );

        let map = ImportResolutionMap::build(&global);

        match map.resolve_reexport_chain(&barrel, "Target") {
            Some(ReExportStep::Named { source_specifier, source_name }) => {
                assert_eq!(source_specifier, "./target");
                assert_eq!(source_name, "Target");
            }
            other => panic!("expected Named, got {:?}", other),
        }
    }

    #[test]
    fn test_reexport_chain_none_for_a_file_with_no_reexports() {
        let barrel = Utf8PathBuf::from("/project/src/no-reexports.ts");
        let global = make_global(vec![], vec![]);
        let map = ImportResolutionMap::build(&global);

        assert!(map.resolve_reexport_chain(&barrel, "Anything").is_none());
    }

    #[test]
    fn test_multiple_files() {
        let file_a = Utf8PathBuf::from("/project/src/A.tsx");
        let file_b = Utf8PathBuf::from("/project/src/B.tsx");
        let global = make_global(
            vec![
                (file_a.clone(), vec![make_binding("Foo", "Foo", "./foo", true)]),
                (file_b.clone(), vec![make_binding("Foo", "Foo", "./other-foo", false)]),
            ],
            vec![],
        );

        let map = ImportResolutionMap::build(&global);

        let a = map.find_import(&file_a, "Foo").expect("A should have Foo");
        assert_eq!(a.specifier, "./foo");
        assert!(a.is_type_only);

        let b = map.find_import(&file_b, "Foo").expect("B should have Foo");
        assert_eq!(b.specifier, "./other-foo");
        assert!(!b.is_type_only);
    }
}
