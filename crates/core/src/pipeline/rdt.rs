//! react-docgen-typescript's `ComponentDoc` shape: the CLI's `--format rdt`, and the `__docgenInfo` Storybook converts
//! to argTypes, which the NAPI binding hands the Storybook integration.

use crate::types::{ComponentEntry, ExtractionOutput, ParsedProp, PropType};

/// RDT's type-name convention for literal unions: `{"name": "enum", "value": [...]}`
/// instead of inlining the literal text into `type.name` as a plain string. Lets
/// prop-table renderers that pattern-match `type.name === "enum"` (a common
/// Storybook addon integration point) show a `<select>` control for the most
/// common, most curated props in any design system (variant, size, color…).
pub fn type_json(prop_type: &PropType) -> serde_json::Value {
    if prop_type.is_literal_union() {
        let values: Vec<serde_json::Value> = match prop_type {
            PropType::Union(members) => members.iter().map(|m| serde_json::json!({"value": m.raw_string()})).collect(),
            PropType::LiteralUnion { members, .. } => {
                members.iter().map(|m| serde_json::json!({"value": format!("\"{m}\"")})).collect()
            }
            _ => vec![],
        };
        return serde_json::json!({ "name": "enum", "raw": prop_type.raw_string(), "value": values });
    }
    serde_json::json!({ "name": prop_type.raw_string() })
}

/// RDT's default `skipChildrenPropWithoutDoc`: an undocumented `children` says nothing a
/// prop table can show, so RDT never emits it. Canonical output keeps it.
pub fn omits(prop: &ParsedProp) -> bool {
    prop.name == "children" && prop.description.is_empty()
}

/// One component as RDT documents it, except that an undocumented `children` stays: this is the `__docgenInfo`
/// Storybook's React renderer converts, and its own docgen lists `children`. `methods` is always `[]` — this tool
/// doesn't extract class methods, and Storybook only reads it for class components.
pub fn docgen_info(entry: &ComponentEntry) -> serde_json::Value {
    component_doc(entry, |_| true)
}

fn component_doc(entry: &ComponentEntry, keep: impl Fn(&ParsedProp) -> bool) -> serde_json::Value {
    let props: serde_json::Map<String, serde_json::Value> = entry
        .props
        .iter()
        .filter(|(_, prop)| keep(prop))
        .map(|(k, prop)| {
            let obj = serde_json::json!({
                "name": prop.name,
                "type": type_json(&prop.prop_type),
                "required": prop.required,
                "defaultValue": prop.default_value.as_ref().map(|d| serde_json::json!({"value": d.value, "computed": d.computed})),
                "description": prop.description,
                "parent": prop.parent.as_ref().map(|p| serde_json::json!({"name": p.name, "fileName": p.file_name})),
            });
            (k.clone(), obj)
        })
        .collect();
    serde_json::json!({
        "displayName": entry.display_name,
        "props": serde_json::Value::Object(props),
        "description": entry.description,
        "methods": [],
        "tags": entry.tags,
        "composes": entry.composes,
    })
}

/// Every component of `output` as RDT documents it, keyed as in the output, pretty-printed.
pub fn serialize(output: &ExtractionOutput) -> serde_json::Result<String> {
    let map: serde_json::Map<String, serde_json::Value> = output
        .components
        .iter()
        .map(|(name, entry)| (name.clone(), component_doc(entry, |prop| !omits(prop))))
        .collect();
    serde_json::to_string_pretty(&serde_json::Value::Object(map))
}
