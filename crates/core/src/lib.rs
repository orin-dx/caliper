//! The core of caliper-docgen: it reads the props of React components from TypeScript source with the OXC parser, and
//! without the TypeScript compiler.
//!
//! [`extract`] runs the pipeline once over a set of source directories and returns an [`ExtractionOutput`].
//! [`pipeline::WatchSession`] keeps a revision of that output up to date as files change, or answers for one
//! component at a time. Problems are reported as [`Diagnostic`]s in the output, not as errors.

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
pub use toon::{render_component_slice_toon, render_component_toon, render_output_toon};
pub use types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
pub use types::output::ExtractionOutput;
