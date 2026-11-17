//! react-docgen-typescript's `ComponentDoc` shape: the CLI's `--format rdt`, and the `__docgenInfo` Storybook converts
//! to argTypes, which the NAPI binding hands the Storybook integration.

use std::io::Write;
use std::sync::Arc;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Serialize, Serializer};
use serde_json::value::RawValue;

use crate::types::{ComponentEntry, ExtractionOutput, ParsedProp, PropType};

/// RDT's type-name convention for literal unions: `{"name": "enum", "value": [...]}`
/// instead of inlining the literal text into `type.name` as a plain string. Lets
/// prop-table renderers that pattern-match `type.name === "enum"` (a common
/// Storybook addon integration point) show a `<select>` control for the most
/// common, most curated props in any design system (variant, size, color…).
/// A single string or number literal is a one-option enum, as Storybook's own docgen has it. An enum member's option is
/// its value (`"small"` for `Size.Small`, `PropType::option_text`), which is what the control passes.
///
/// Storybook unquotes each option's source text into a string or a number, so `true` in an enum would reach the control
/// as the string "true". A union with a boolean member therefore stays a type name, which Storybook's converter splits
/// at the `|` and parses as JSON into the options `["a", 1, true]`, unless a member's text would not survive that (a
/// string holding `|`, `"` or `\`); it is then an enum, so that no option is lost.
pub fn type_json(prop_type: &PropType) -> serde_json::Value {
    if prop_type.is_literal_union() {
        let values: Vec<serde_json::Value> = match prop_type {
            PropType::Union(members) => members.iter().map(|m| serde_json::json!({"value": m.option_text()})).collect(),
            PropType::LiteralUnion { members, .. } => {
                members.iter().map(|m| serde_json::json!({"value": format!("\"{m}\"")})).collect()
            }
            single => vec![serde_json::json!({"value": single.option_text()})],
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

/// One prop as RDT documents it.
fn prop_json(prop: &ParsedProp) -> serde_json::Value {
    serde_json::json!({
        "name": prop.name,
        "type": type_json(&prop.prop_type),
        "required": prop.required,
        "defaultValue": prop.default_value.as_ref().map(|d| serde_json::json!({"value": d.value, "computed": d.computed})),
        "description": prop.description,
        "parent": prop.parent.as_ref().map(|p| serde_json::json!({"name": p.name, "fileName": p.file_name})),
    })
}

/// The props a props table lists: the component's own and inherited ones, then the curated attributes inherited from an
/// HTML element, which `curated` mode keeps apart so the canonical output can collapse them. A prop named in both is
/// listed once, as the component declares it. A prop tagged `@ignore` is left out, as Storybook's own docgen leaves it
/// out; the canonical output keeps it.
fn listed_props<'a>(
    entry: &'a ComponentEntry,
    keep: impl Fn(&ParsedProp) -> bool + 'a,
) -> impl Iterator<Item = (&'a String, &'a Arc<ParsedProp>)> {
    entry
        .props
        .iter()
        .chain(entry.notable_inherited.iter().filter(|(name, _)| !entry.props.contains_key(*name)))
        .filter(move |(_, prop)| keep(prop) && !prop.tags.contains_key("ignore"))
}

/// One component as RDT documents it, except that an undocumented `children` stays: this is the `__docgenInfo`
/// Storybook's React renderer converts, and its own docgen lists `children`. `methods` is always `[]` — this tool
/// doesn't extract class methods, and Storybook only reads it for class components.
pub fn docgen_info(entry: &ComponentEntry) -> serde_json::Value {
    let props: serde_json::Map<String, serde_json::Value> =
        listed_props(entry, |_| true).map(|(k, prop)| (k.clone(), prop_json(prop))).collect();
    serde_json::json!({
        "displayName": entry.display_name,
        "props": serde_json::Value::Object(props),
        "description": entry.description,
        "methods": [],
        "tags": entry.tags,
        "composes": entry.composes,
    })
}

/// Indentation a prop's lines take in the pretty-printed document: it sits in a component, in that component's
/// `props`, in the document: three objects deep, two spaces each.
const PROP_INDENT: &str = "      ";

/// A prop as the document holds it, rendered once. Pretty printing a prop alone indents by its own depth only, and a
/// JSON string holds no raw newline, so every newline is a line break of the layout and the document's nesting is all
/// that is left to add.
fn render_prop(prop: &ParsedProp) -> serde_json::Result<Box<RawValue>> {
    let pretty = serde_json::to_string_pretty(&prop_json(prop))?;
    let mut indented = String::with_capacity(pretty.len() + PROP_INDENT.len() * 8);
    for (i, line) in pretty.split('\n').enumerate() {
        if i > 0 {
            indented.push('\n');
            indented.push_str(PROP_INDENT);
        }
        indented.push_str(line);
    }
    RawValue::from_string(indented)
}

/// Rendered props by the address of the allocation that holds them.
type Rendered = FxHashMap<usize, Box<RawValue>>;

fn address(prop: &Arc<ParsedProp>) -> usize {
    Arc::as_ptr(prop) as usize
}

/// The listed props of one component, each already rendered.
struct Props<'a> {
    entry: &'a ComponentEntry,
    rendered: &'a Rendered,
}

impl Serialize for Props<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (name, prop) in listed_props(self.entry, |prop| !omits(prop)) {
            map.serialize_entry(name, &self.rendered[&address(prop)])?;
        }
        map.end()
    }
}

struct Doc<'a> {
    entry: &'a ComponentEntry,
    rendered: &'a Rendered,
}

impl Serialize for Doc<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let entry = self.entry;
        let mut doc = serializer.serialize_struct("ComponentDoc", 7)?;
        doc.serialize_field("displayName", &entry.display_name)?;
        doc.serialize_field("props", &Props { entry, rendered: self.rendered })?;
        doc.serialize_field("description", &entry.description)?;
        doc.serialize_field("methods", &[(); 0])?;
        doc.serialize_field("tags", &entry.tags)?;
        doc.serialize_field("composes", &entry.composes)?;
        doc.serialize_field("filePath", &entry.file_path)?;
        doc.end()
    }
}

struct Docs<'a> {
    components: Vec<(&'a String, &'a ComponentEntry)>,
    rendered: Rendered,
}

impl Serialize for Docs<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.components.len()))?;
        for (name, entry) in &self.components {
            map.serialize_entry(name, &Doc { entry, rendered: &self.rendered })?;
        }
        map.end()
    }
}

/// Render the props of the components `output` documents, once each.
fn documents(output: &ExtractionOutput, include_unexported: bool) -> serde_json::Result<Docs<'_>> {
    let components: Vec<_> =
        output.components.iter().filter(|(_, entry)| include_unexported || entry.exported).collect();
    let mut distinct: FxHashMap<usize, &Arc<ParsedProp>> = FxHashMap::default();
    for (_, entry) in &components {
        for (_, prop) in listed_props(entry, |prop| !omits(prop)) {
            distinct.entry(address(prop)).or_insert(prop);
        }
    }
    let rendered = distinct
        .into_par_iter()
        .map(|(address, prop)| Ok((address, render_prop(prop)?)))
        .collect::<serde_json::Result<_>>()?;
    Ok(Docs { components, rendered })
}

/// The components of `output` as RDT documents them, keyed as in the output, pretty-printed to `writer` with no
/// trailing newline. Like RDT, only exported components unless `include_unexported`, each with RDT's `filePath`, which
/// `__docgenInfo` doesn't carry.
///
/// Components that inherit one interface share its props' allocations, so a project's few thousand distinct props occur
/// half a million times: each is rendered once and its bytes written wherever it occurs, and the document is never held
/// whole.
pub fn write(output: &ExtractionOutput, include_unexported: bool, writer: impl Write) -> serde_json::Result<()> {
    serde_json::to_writer_pretty(writer, &documents(output, include_unexported)?)
}

/// [`write()`] to a string.
pub fn serialize(output: &ExtractionOutput, include_unexported: bool) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&documents(output, include_unexported)?)
}

#[cfg(test)]
mod tests;
