//! Builders for the checker's answer types, so a field added to them breaks one place rather than every test.

use std::sync::Arc;

use caliper_docgen_core::types::PropType;
use caliper_docgen_core::types::semantic::{SemanticComponent, SemanticProp};

/// A prop with no description, tags or declaring interface.
pub fn semantic_prop(name: &str, prop_type: PropType, required: bool, react_declared: bool) -> Arc<SemanticProp> {
    Arc::new(SemanticProp {
        name: name.to_string(),
        prop_type,
        required,
        description: String::new(),
        react_declared,
        tags: Default::default(),
        parent: None,
        declarations: Vec::new(),
    })
}

/// A component whose props type is made of no `@types/react` types.
pub fn semantic_component(key: &str, props: Vec<Arc<SemanticProp>>) -> SemanticComponent {
    SemanticComponent { key: key.to_string(), props, composes: Some(Vec::new()) }
}
