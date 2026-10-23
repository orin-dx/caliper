//! Plugin system for extending component prop extraction & post-processing.
//!
//! Inspired by `michi`'s zero-dep trait architecture and `callisto`'s modular design.
//! Plugins can hook into extraction events to transform AST data or enrich component entries.

use std::sync::Arc;

use crate::types::{ComponentEntry, SourceData};

/// Hook trait for docgen extension plugins.
pub trait DocgenPlugin: Send + Sync {
    /// Unique plugin name identifier.
    fn name(&self) -> &str;

    /// Invoked after AST extraction for a single file (`SourceData`).
    fn on_file_extracted(&self, _file_path: &camino::Utf8Path, _data: &mut SourceData) {}

    /// Invoked after a component has been fully resolved into a `ComponentEntry`.
    fn on_component_resolved(&self, _entry: &mut ComponentEntry) {}
}

impl std::fmt::Debug for dyn DocgenPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocgenPlugin").field("name", &self.name()).finish()
    }
}

/// A registry holding active plugins for a pipeline run.
#[derive(Debug, Default, Clone)]
pub struct PluginRegistry {
    plugins: Vec<Arc<dyn DocgenPlugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self { plugins: Vec::new() }
    }

    pub fn register<P: DocgenPlugin + 'static>(&mut self, plugin: P) {
        self.plugins.push(Arc::new(plugin));
    }

    pub fn register_arc(&mut self, plugin: Arc<dyn DocgenPlugin>) {
        self.plugins.push(plugin);
    }

    pub fn run_on_file_extracted(
        &self,
        file_path: &camino::Utf8Path,
        data: &mut SourceData,
    ) -> Vec<crate::types::Diagnostic> {
        let mut diagnostics = Vec::new();
        for plugin in &self.plugins {
            let label = format!("plugin:{}:on_file_extracted", plugin.name());
            if let Err(diag) = crate::panic_guard::contain_panic(&label, || plugin.on_file_extracted(file_path, data)) {
                diagnostics.push(diag);
            }
        }
        diagnostics
    }

    pub fn run_on_component_resolved(&self, entry: &mut ComponentEntry) -> Vec<crate::types::Diagnostic> {
        let mut diagnostics = Vec::new();
        for plugin in &self.plugins {
            let label = format!("plugin:{}:on_component_resolved", plugin.name());
            if let Err(diag) = crate::panic_guard::contain_panic(&label, || plugin.on_component_resolved(entry)) {
                diagnostics.push(diag);
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ComponentEntry;

    struct TestEnricherPlugin;

    impl DocgenPlugin for TestEnricherPlugin {
        fn name(&self) -> &str {
            "test-enricher"
        }

        fn on_component_resolved(&self, entry: &mut ComponentEntry) {
            entry.composes.push("TestEnricher".into());
        }
    }

    #[test]
    fn test_plugin_registry_hook() {
        let mut registry = PluginRegistry::new();
        registry.register(TestEnricherPlugin);

        let mut entry = ComponentEntry {
            display_name: "Button".into(),
            file_path: "src/Button.tsx".into(),
            props: Default::default(),
            description: String::new(),
            inheritance: vec![],
            notable_inherited: Default::default(),
            discriminant_prop: None,
            composes: vec![],
            tags: Default::default(),
            methods: vec![],
            detected_by: crate::types::DetectionRule::FunctionDeclaration,
            exported: true,
        };

        registry.run_on_component_resolved(&mut entry);
        assert_eq!(entry.composes, vec!["TestEnricher"]);
    }

    struct FileHookPlugin;
    impl DocgenPlugin for FileHookPlugin {
        fn name(&self) -> &str {
            "file-hook"
        }
        fn on_file_extracted(&self, _path: &camino::Utf8Path, data: &mut SourceData) {
            data.interfaces.insert(
                "HookIface".into(),
                crate::types::CollectedInterface {
                    name: "HookIface".into(),
                    file_path: "src/test.tsx".into(),
                    scoped_key: "src/test.tsx:HookIface".into(),
                    props: vec![],
                    extends: vec![],
                    description: String::new(),
                    tags: Default::default(),
                },
            );
        }
    }

    #[test]
    fn test_plugin_registry_multiple_plugins_chained() {
        let mut registry = PluginRegistry::new();
        registry.register(TestEnricherPlugin);
        registry.register_arc(std::sync::Arc::new(FileHookPlugin));

        let mut data = SourceData::default();
        registry.run_on_file_extracted(camino::Utf8Path::new("src/test.tsx"), &mut data);
        assert!(data.interfaces.contains_key("HookIface"));
    }

    #[test]
    fn a_panicking_plugin_is_contained_and_tagged_with_its_name_others_still_run() {
        struct PanickingPlugin;
        impl DocgenPlugin for PanickingPlugin {
            fn name(&self) -> &str {
                "panicking-plugin"
            }
            fn on_component_resolved(&self, _entry: &mut ComponentEntry) {
                panic!("boom");
            }
        }

        let mut registry = PluginRegistry::new();
        registry.register(PanickingPlugin);
        registry.register(TestEnricherPlugin);

        let mut entry = ComponentEntry {
            display_name: "Button".into(),
            file_path: "src/Button.tsx".into(),
            props: Default::default(),
            description: String::new(),
            inheritance: vec![],
            notable_inherited: Default::default(),
            discriminant_prop: None,
            composes: vec![],
            tags: Default::default(),
            methods: vec![],
            detected_by: crate::types::DetectionRule::FunctionDeclaration,
            exported: true,
        };

        let diagnostics = registry.run_on_component_resolved(&mut entry);

        assert_eq!(diagnostics.len(), 1, "expected exactly one diagnostic, from the panicking plugin");
        assert_eq!(diagnostics[0].code, crate::types::DiagnosticCode::InternalPanic);
        assert!(
            diagnostics[0].message.contains("panicking-plugin"),
            "diagnostic should name the panicking plugin, got {}",
            diagnostics[0].message
        );
        assert_eq!(
            entry.composes,
            vec!["TestEnricher"],
            "the second, well-behaved plugin should still run after the first one panicked"
        );
    }
}
