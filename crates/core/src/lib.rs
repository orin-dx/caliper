pub(crate) mod cache;
pub(crate) mod dependency_types;
pub(crate) mod extractor;
pub(crate) mod import_map;
pub(crate) mod known;
pub(crate) mod named_type_index;
pub mod panic_guard;
pub(crate) mod path_key;
pub mod pipeline;
pub mod plugin;
pub mod react_types;
pub(crate) mod resolver;
pub mod toon;
pub mod types;

// Re-export the primary consumer API at the crate root.
pub use pipeline::{extract, PipelineOptions};
pub use plugin::{DocgenPlugin, PluginRegistry};
pub use toon::{render_component_toon, render_output_toon};
pub use types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
pub use types::output::ExtractionOutput;
