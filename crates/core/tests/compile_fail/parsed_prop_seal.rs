// SPEC-TYPES-001 AC-001: a bare `ParsedProp { .. }` struct literal must fail
// to compile outside crates/core/src/types/output.rs — the private `_seal`
// field is what enforces this. This file lives outside the crate entirely
// (trybuild compiles it as its own crate), the strictest possible test of
// "outside this module."
use caliper_docgen_core::types::{ParsedProp, PropType};

fn main() {
    let _ = ParsedProp {
        name: "x".to_string(),
        prop_type: PropType::String,
        required: false,
        default_value: None,
        description: String::new(),
        tags: Default::default(),
        parent: None,
        declarations: vec![],
    };
}
