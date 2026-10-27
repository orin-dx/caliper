//! Shared data types for caliper-docgen.
//!
//! These types form the contract between pipeline stages.
//! Rules:
//! - `CompactString` for names/type strings (avoids heap alloc for short strings)
//! - `BTreeMap` for JSON-facing output (deterministic key ordering)
//! - `FxHashMap` for internal lookup maps (performance)
//! - All types are `Send + Sync` — required for rayon and NAPI
//!
//! # Module layout
//! - `collected` — raw AST-level types produced by the extractor
//! - `output`    — semantic output types produced by the resolver
//! - `diagnostic`— Diagnostic, DiagnosticSeverity, DiagnosticCode
//! - `global`    — GlobalSourceData + ResolveState (the shared resolution context)

pub mod collected;
pub mod diagnostic;
pub mod global;
pub mod output;
pub mod semantic;

// Flat re-export so existing `use crate::types::*` keeps working unchanged.
pub use collected::*;
pub use diagnostic::*;
pub use global::*;
pub use output::*;
