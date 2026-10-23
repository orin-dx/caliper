//! The canonical JSON of an [`ExtractionOutput`] (ADR 0009): each distinct prop is written once, in a top-level `props`
//! table, and a component's `props` and `notableInherited` map a prop name to its id. The Rust types keep props inline.

use std::collections::BTreeMap;
use std::fmt;

use rayon::prelude::*;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{ComponentEntry, ExtractionOutput, ExtractionStats, ParsedProp};
use crate::types::{Diagnostic, EnumEntry};

/// A prop's id in the table: the 64-bit FNV-1a hash of its compact JSON, written as 16 hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PropId(u64);

impl PropId {
    fn of(json: &[u8]) -> PropId {
        let hash = json.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        PropId(hash)
    }
}

impl fmt::Display for PropId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl Serialize for PropId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// The ids of one component's props and notable inherited props, by prop name.
pub(crate) struct ComponentIds<'a> {
    props: Vec<(&'a str, PropId)>,
    notable_inherited: Vec<(&'a str, PropId)>,
}

/// The distinct props of a set of components, each with the JSON its id was computed from.
pub(crate) struct PropTable<'a> {
    entries: BTreeMap<PropId, (&'a ParsedProp, Vec<u8>)>,
}

impl Serialize for PropTable<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.entries.iter().map(|(id, (prop, _))| (id, prop)))
    }
}

/// A prop by name, with its id and the JSON the id was computed from.
type Row<'a> = (&'a str, PropId, &'a ParsedProp, Vec<u8>);

/// Identify every prop of `entries` and collect the distinct ones.
pub(crate) fn intern<'a, E: serde::ser::Error>(
    entries: &[&'a ComponentEntry],
) -> Result<(Vec<ComponentIds<'a>>, PropTable<'a>), E> {
    let identify = |props: &'a BTreeMap<String, ParsedProp>| -> serde_json::Result<Vec<Row<'a>>> {
        props
            .iter()
            .map(|(name, prop)| {
                let json = serde_json::to_vec(prop)?;
                Ok((name.as_str(), PropId::of(&json), prop, json))
            })
            .collect()
    };
    let rows: Vec<(Vec<Row<'a>>, Vec<Row<'a>>)> = entries
        .par_iter()
        .map(|entry| Ok((identify(&entry.props)?, identify(&entry.notable_inherited)?)))
        .collect::<serde_json::Result<_>>()
        .map_err(E::custom)?;

    let mut table = PropTable { entries: BTreeMap::new() };
    let ids = rows
        .into_iter()
        .map(|(props, notable_inherited)| {
            Ok(ComponentIds { props: table.add(props)?, notable_inherited: table.add(notable_inherited)? })
        })
        .collect::<Result<_, E>>()?;
    Ok((ids, table))
}

impl<'a> PropTable<'a> {
    /// Add each row's prop under its id, and return the ids by name. Fails if a different prop already holds an id, so
    /// one can never stand in for the other.
    fn add<E: serde::ser::Error>(&mut self, rows: Vec<Row<'a>>) -> Result<Vec<(&'a str, PropId)>, E> {
        rows.into_iter()
            .map(|(name, id, prop, json)| {
                match self.entries.get(&id) {
                    Some((_, existing)) if *existing != json => {
                        return Err(E::custom(format!(
                            "prop '{name}' and a different prop have the same id {id}: {} and {}",
                            String::from_utf8_lossy(existing),
                            String::from_utf8_lossy(&json)
                        )));
                    }
                    Some(_) => {}
                    None => {
                        self.entries.insert(id, (prop, json));
                    }
                }
                Ok((name, id))
            })
            .collect()
    }
}

/// A component as the canonical JSON writes it: every field as `ComponentEntry` derives it, except that `props` and
/// `notableInherited` hold ids.
pub(crate) struct InternedComponent<'a> {
    pub(crate) entry: &'a ComponentEntry,
    pub(crate) ids: &'a ComponentIds<'a>,
}

struct IdMap<'a>(&'a [(&'a str, PropId)]);

impl Serialize for IdMap<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(name, id)| (name, id)))
    }
}

impl Serialize for InternedComponent<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Destructured so a new field fails to compile here instead of going missing from the JSON.
        let ComponentEntry {
            display_name,
            file_path,
            description,
            props: _,
            inheritance,
            notable_inherited: _,
            discriminant_prop,
            composes,
            tags,
            methods,
            detected_by,
            exported,
        } = self.entry;
        let mut out = serializer.serialize_struct("ComponentEntry", 12)?;
        out.serialize_field("displayName", display_name)?;
        out.serialize_field("filePath", file_path)?;
        out.serialize_field("description", description)?;
        out.serialize_field("props", &IdMap(&self.ids.props))?;
        out.serialize_field("inheritance", inheritance)?;
        out.serialize_field("notableInherited", &IdMap(&self.ids.notable_inherited))?;
        out.serialize_field("discriminantProp", discriminant_prop)?;
        out.serialize_field("composes", composes)?;
        out.serialize_field("tags", tags)?;
        out.serialize_field("methods", methods)?;
        out.serialize_field("detectedBy", detected_by)?;
        out.serialize_field("exported", exported)?;
        out.end()
    }
}

struct InternedComponents<'a> {
    keys: Vec<&'a str>,
    entries: Vec<&'a ComponentEntry>,
    ids: Vec<ComponentIds<'a>>,
}

impl Serialize for InternedComponents<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let components = self.entries.iter().zip(&self.ids).map(|(entry, ids)| InternedComponent { entry, ids });
        serializer.collect_map(self.keys.iter().zip(components))
    }
}

impl Serialize for ExtractionOutput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let ExtractionOutput { components, enums, diagnostics, stats } = self;
        let entries: Vec<&ComponentEntry> = components.values().collect();
        let (ids, props) = intern::<S::Error>(&entries)?;
        let components = InternedComponents { keys: components.keys().map(String::as_str).collect(), entries, ids };
        let mut out = serializer.serialize_struct("ExtractionOutput", 5)?;
        out.serialize_field("components", &components)?;
        out.serialize_field("props", &props)?;
        out.serialize_field("enums", enums)?;
        out.serialize_field("diagnostics", diagnostics)?;
        out.serialize_field("stats", stats)?;
        out.end()
    }
}

impl<'de> Deserialize<'de> for ExtractionOutput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            components: BTreeMap<String, serde_json::Value>,
            props: BTreeMap<String, serde_json::Value>,
            enums: BTreeMap<String, Vec<EnumEntry>>,
            diagnostics: Vec<Diagnostic>,
            stats: ExtractionStats,
        }
        let Wire { components, props, enums, diagnostics, stats } = Wire::deserialize(deserializer)?;
        let components = components
            .into_iter()
            .map(|(key, component)| Ok((key, expand(component, &props).map_err(D::Error::custom)?)))
            .collect::<Result<_, D::Error>>()?;
        Ok(ExtractionOutput { components, enums, diagnostics, stats })
    }
}

/// Replace a component's prop ids with the props they name.
fn expand(
    mut component: serde_json::Value,
    table: &BTreeMap<String, serde_json::Value>,
) -> Result<ComponentEntry, String> {
    for field in ["props", "notableInherited"] {
        let Some(serde_json::Value::Object(props)) = component.get_mut(field) else { continue };
        for value in props.values_mut() {
            let id = value.as_str().ok_or_else(|| format!("`{field}` holds {value} where a prop id belongs"))?;
            *value = table.get(id).cloned().ok_or_else(|| format!("prop id {id} is not in the `props` table"))?;
        }
    }
    serde_json::from_value(component).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
