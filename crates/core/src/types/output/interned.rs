//! The canonical JSON of an [`ExtractionOutput`] (ADR 0009, ADR 0018): each distinct prop is written once, in a
//! top-level `props` table. A component's `props` lists the ids of prop sets, each a table entry that maps the names of
//! the props one declaring type contributes to prop ids; its inheritance layers' `omitted` lists are ids into a
//! `nameLists` table; its `notableInherited` maps names to prop ids. The Rust types keep all of it inline.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{ComponentEntry, ExtractionOutput, ExtractionStats, InheritedLayer, ParsedProp};
use crate::types::{Diagnostic, EnumEntry};

/// A prop's id in the table: the 64-bit FNV-1a hash of its compact JSON, written as 16 hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PropId(u64);

/// The id of a prop set in the `propSets` table: the hash of its (name, prop id) entries, in name order.
type SetId = PropId;

/// The id of a list in the `nameLists` table: the hash of its names, in order.
type ListId = PropId;

/// The (prop name, prop id) entries of one prop set, in name order.
type Members<'a> = Vec<(&'a str, PropId)>;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

impl PropId {
    fn of(json: &[u8]) -> PropId {
        PropId(fnv(FNV_OFFSET, json))
    }

    /// Names and ids are separated by bytes no JSON string holds unescaped, so `[("a", x)]` never hashes as another set.
    fn of_set(entries: &[(&str, PropId)]) -> SetId {
        PropId(
            entries.iter().fold(FNV_OFFSET, |hash, (name, id)| {
                fnv(fnv(fnv(hash, name.as_bytes()), &[0xff]), &id.0.to_le_bytes())
            }),
        )
    }

    fn of_list(names: &[String]) -> ListId {
        PropId(names.iter().fold(FNV_OFFSET, |hash, name| fnv(fnv(hash, name.as_bytes()), &[0xff])))
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

/// What a component's JSON refers to by id: its prop sets, its notable inherited props by name, and one omitted-names
/// list per inheritance layer, in layer order.
pub(crate) struct ComponentIds<'a> {
    props: Vec<SetId>,
    notable_inherited: Vec<(&'a str, PropId)>,
    omitted: Vec<ListId>,
}

/// The tables a set of components refer to: their distinct props, prop sets and omitted-names lists.
pub(crate) struct Tables<'a> {
    props: PropTable<'a>,
    prop_sets: BTreeMap<SetId, Members<'a>>,
    name_lists: NameLists<'a>,
}

impl<'a> Tables<'a> {
    /// Write `props`, `propSets` and `nameLists` as the next fields of `out`.
    pub(crate) fn write<T: SerializeStruct>(&self, out: &mut T) -> Result<(), T::Error> {
        out.serialize_field("props", &self.props)?;
        out.serialize_field("propSets", &SetTable(&self.prop_sets))?;
        out.serialize_field("nameLists", &self.name_lists)
    }

    /// Add the lists `diagnostics` name in `affects` to the `nameLists` table, and identify them.
    pub(crate) fn intern_affects<E: serde::ser::Error>(
        &mut self,
        diagnostics: &'a [Diagnostic],
    ) -> Result<InternedDiagnostics<'a>, E> {
        InternedDiagnostics::new(&mut self.name_lists, diagnostics)
    }

    /// How many fields [`Tables::write`] adds to a struct.
    pub(crate) const FIELDS: usize = 3;
}

/// The omitted or affected names the JSON refers to by id, each list written once.
pub(crate) type NameLists<'a> = BTreeMap<ListId, &'a [String]>;

/// Add `names` to `lists` and return its id. Fails if a different list already holds the id.
fn add_list<'a, E: serde::ser::Error>(lists: &mut NameLists<'a>, names: &'a [String]) -> Result<ListId, E> {
    let id = PropId::of_list(names);
    match lists.get(&id) {
        Some(existing) if *existing != names => {
            Err(E::custom(format!("two different name lists have the same id {id}: {existing:?} and {names:?}")))
        }
        Some(_) => Ok(id),
        None => {
            lists.insert(id, names);
            Ok(id)
        }
    }
}

/// Diagnostics as the canonical JSON writes them: every field as `Diagnostic` derives, except that `affects` is the id
/// of a list in the `nameLists` table, and absent when the list is empty.
pub(crate) struct InternedDiagnostics<'a> {
    diagnostics: &'a [Diagnostic],
    affects: Vec<Option<ListId>>,
}

impl<'a> InternedDiagnostics<'a> {
    /// Identify `diagnostics`' `affects` lists, adding them to `lists`.
    pub(crate) fn new<E: serde::ser::Error>(
        lists: &mut NameLists<'a>,
        diagnostics: &'a [Diagnostic],
    ) -> Result<Self, E> {
        let affects =
            diagnostics
                .iter()
                .map(|diagnostic| {
                    if diagnostic.affects.is_empty() {
                        Ok(None)
                    } else {
                        add_list(lists, &diagnostic.affects).map(Some)
                    }
                })
                .collect::<Result<_, E>>()?;
        Ok(InternedDiagnostics { diagnostics, affects })
    }
}

struct InternedDiagnostic<'a> {
    diagnostic: &'a Diagnostic,
    affects: Option<ListId>,
}

impl Serialize for InternedDiagnostic<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Destructured so a new field fails to compile here instead of going missing from the JSON.
        let Diagnostic { severity, message, file, line, column, help, code, affects: _ } = self.diagnostic;
        let mut out = serializer.serialize_struct("Diagnostic", 7 + usize::from(self.affects.is_some()))?;
        out.serialize_field("severity", severity)?;
        out.serialize_field("message", message)?;
        out.serialize_field("file", file)?;
        out.serialize_field("line", line)?;
        out.serialize_field("column", column)?;
        out.serialize_field("help", help)?;
        out.serialize_field("code", code)?;
        if let Some(affects) = &self.affects {
            out.serialize_field("affects", affects)?;
        }
        out.end()
    }
}

impl Serialize for InternedDiagnostics<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(
            self.diagnostics
                .iter()
                .zip(&self.affects)
                .map(|(diagnostic, &affects)| InternedDiagnostic { diagnostic, affects }),
        )
    }
}

struct SetTable<'a>(&'a BTreeMap<SetId, Vec<(&'a str, PropId)>>);

impl Serialize for SetTable<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(id, entries)| (id, IdMap(entries))))
    }
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

/// Identify every prop of `entries`, collect the distinct ones, and group each component's props into prop sets by the
/// type that declares them. Components that inherit one interface share its props, so each shared allocation is
/// serialized once, and share its prop set, so the set is written once.
pub(crate) fn intern<'a, E: serde::ser::Error>(
    entries: &[&'a ComponentEntry],
) -> Result<(Vec<ComponentIds<'a>>, Tables<'a>), E> {
    let mut slots: FxHashMap<*const ParsedProp, usize> = FxHashMap::default();
    let mut allocations: Vec<&'a ParsedProp> = Vec::new();
    let mut slots_of = |props: &'a BTreeMap<String, Arc<ParsedProp>>| -> Vec<(&'a str, usize)> {
        props
            .iter()
            .map(|(name, prop)| {
                let slot = *slots.entry(Arc::as_ptr(prop)).or_insert_with(|| {
                    allocations.push(prop);
                    allocations.len() - 1
                });
                (name.as_str(), slot)
            })
            .collect()
    };
    let named: Vec<_> =
        entries.iter().map(|entry| (slots_of(&entry.props), slots_of(&entry.notable_inherited))).collect();

    let identified: Vec<(PropId, Vec<u8>)> = allocations
        .par_iter()
        .map(|prop| {
            let json = serde_json::to_vec(prop)?;
            Ok((PropId::of(&json), json))
        })
        .collect::<serde_json::Result<_>>()
        .map_err(E::custom)?;

    let mut table = PropTable { entries: BTreeMap::new() };
    let mut ids = Vec::with_capacity(identified.len());
    for (prop, (id, json)) in allocations.iter().copied().zip(identified) {
        table.add(prop, id, json)?;
        ids.push(id);
    }

    let grouped: Vec<Vec<(SetId, Members<'a>)>> =
        named.par_iter().map(|(props, _)| group_into_sets(props, &allocations, &ids)).collect();

    let mut prop_sets: BTreeMap<SetId, Members<'a>> = BTreeMap::new();
    let mut name_lists = NameLists::new();
    let mut components = Vec::with_capacity(entries.len());
    for ((entry, groups), (_, notable)) in entries.iter().zip(grouped).zip(named) {
        let mut set_ids = Vec::with_capacity(groups.len());
        for (id, members) in groups {
            match prop_sets.get(&id) {
                Some(existing) if *existing != members => {
                    return Err(E::custom(format!(
                        "two different prop sets have the same id {id}: {existing:?} and {members:?}"
                    )));
                }
                Some(_) => {}
                None => {
                    prop_sets.insert(id, members);
                }
            }
            set_ids.push(id);
        }
        let omitted = entry
            .inheritance
            .iter()
            .map(|layer| add_list(&mut name_lists, &layer.omitted))
            .collect::<Result<Vec<_>, E>>()?;
        let by_name = |slots: Vec<(&'a str, usize)>| -> Vec<(&'a str, PropId)> {
            slots.into_iter().map(|(name, slot)| (name, ids[slot])).collect()
        };
        components.push(ComponentIds { props: set_ids, notable_inherited: by_name(notable), omitted });
    }
    Ok((components, Tables { props: table, prop_sets, name_lists }))
}

/// Split a component's props by the type that declares them, and identify each group. The declaring type is the unit
/// components share, so a group that two components inherit is the same set in both. A prop with no declaring type
/// forms a group of its own, first.
fn group_into_sets<'a>(
    props: &[(&'a str, usize)],
    allocations: &[&ParsedProp],
    ids: &[PropId],
) -> Vec<(SetId, Members<'a>)> {
    let mut groups: BTreeMap<Option<(&str, &str)>, Members<'a>> = BTreeMap::new();
    for &(name, slot) in props {
        let parent = allocations[slot].parent.as_ref().map(|parent| (parent.name.as_str(), parent.file_name.as_str()));
        groups.entry(parent).or_default().push((name, ids[slot]));
    }
    groups.into_values().map(|members| (PropId::of_set(&members), members)).collect()
}

impl<'a> PropTable<'a> {
    /// Add `prop` under `id`, the hash of `json`. Fails if a different prop already holds the id, so one can never stand
    /// in for the other.
    fn add<E: serde::ser::Error>(&mut self, prop: &'a ParsedProp, id: PropId, json: Vec<u8>) -> Result<(), E> {
        match self.entries.get(&id) {
            Some((_, existing)) if *existing != json => Err(E::custom(format!(
                "prop '{}' and a different prop have the same id {id}: {} and {}",
                prop.name,
                String::from_utf8_lossy(existing),
                String::from_utf8_lossy(&json)
            ))),
            Some(_) => Ok(()),
            None => {
                self.entries.insert(id, (prop, json));
                Ok(())
            }
        }
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

/// An inheritance layer as the canonical JSON writes it: every field as `InheritedLayer` derives, except that `omitted`
/// is the id of a list in the `nameLists` table.
struct InternedLayer<'a> {
    layer: &'a InheritedLayer,
    omitted: ListId,
}

impl Serialize for InternedLayer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Destructured so a new field fails to compile here instead of going missing from the JSON.
        let InheritedLayer { type_name, file_name, omitted: _, html_element, total_props } = self.layer;
        let mut out = serializer.serialize_struct("InheritedLayer", 5)?;
        out.serialize_field("typeName", type_name)?;
        out.serialize_field("fileName", file_name)?;
        out.serialize_field("omitted", &self.omitted)?;
        out.serialize_field("htmlElement", html_element)?;
        out.serialize_field("totalProps", total_props)?;
        out.end()
    }
}

struct InternedLayers<'a> {
    layers: &'a [InheritedLayer],
    omitted: &'a [ListId],
}

impl Serialize for InternedLayers<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer
            .collect_seq(self.layers.iter().zip(self.omitted).map(|(layer, &omitted)| InternedLayer { layer, omitted }))
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
        out.serialize_field("props", &self.ids.props)?;
        out.serialize_field("inheritance", &InternedLayers { layers: inheritance, omitted: &self.ids.omitted })?;
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
        let (ids, mut tables) = intern::<S::Error>(&entries)?;
        let diagnostics = tables.intern_affects::<S::Error>(diagnostics)?;
        let components = InternedComponents { keys: components.keys().map(String::as_str).collect(), entries, ids };
        let mut out = serializer.serialize_struct("ExtractionOutput", 5 + Tables::FIELDS)?;
        out.serialize_field("schemaVersion", &super::SCHEMA_VERSION)?;
        out.serialize_field("components", &components)?;
        tables.write(&mut out)?;
        out.serialize_field("enums", enums)?;
        out.serialize_field("diagnostics", &diagnostics)?;
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
            // Output written before the version existed has the shape of version 1.
            #[serde(default = "first_version")]
            schema_version: u32,
            components: BTreeMap<String, serde_json::Value>,
            props: BTreeMap<String, serde_json::Value>,
            prop_sets: BTreeMap<String, BTreeMap<String, String>>,
            name_lists: BTreeMap<String, Vec<String>>,
            enums: BTreeMap<String, Vec<EnumEntry>>,
            diagnostics: Vec<serde_json::Value>,
            stats: ExtractionStats,
        }
        fn first_version() -> u32 {
            1
        }
        let Wire { schema_version, components, props, prop_sets, name_lists, enums, diagnostics, stats } =
            Wire::deserialize(deserializer)?;
        let tables = ReadTables { props: &props, prop_sets: &prop_sets, name_lists: &name_lists };
        if schema_version != super::SCHEMA_VERSION {
            return Err(D::Error::custom(format!(
                "output schema version {schema_version} is not the version {} this build reads",
                super::SCHEMA_VERSION
            )));
        }
        let diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| expand_diagnostic(diagnostic, &name_lists).map_err(D::Error::custom))
            .collect::<Result<_, D::Error>>()?;
        let components = components
            .into_iter()
            .map(|(key, component)| Ok((key, expand(component, &tables).map_err(D::Error::custom)?)))
            .collect::<Result<_, D::Error>>()?;
        Ok(ExtractionOutput { components, enums, diagnostics, stats })
    }
}

/// Replace a diagnostic's `affects` list id with the names it stands for.
pub(crate) fn expand_diagnostic(
    mut diagnostic: serde_json::Value,
    name_lists: &BTreeMap<String, Vec<String>>,
) -> Result<Diagnostic, String> {
    if let Some(affects) = diagnostic.get_mut("affects") {
        let id = affects.as_str().ok_or_else(|| format!("`affects` holds {affects} where a name list id belongs"))?;
        let names = name_lists.get(id).ok_or_else(|| format!("name list id {id} is not in the `nameLists` table"))?;
        *affects = serde_json::json!(names);
    }
    serde_json::from_value(diagnostic).map_err(|e| e.to_string())
}

/// The tables a component's ids point into, as read.
struct ReadTables<'a> {
    props: &'a BTreeMap<String, serde_json::Value>,
    prop_sets: &'a BTreeMap<String, BTreeMap<String, String>>,
    name_lists: &'a BTreeMap<String, Vec<String>>,
}

/// Replace a component's prop set ids and omitted-list ids with the props and names they stand for.
fn expand(mut component: serde_json::Value, tables: &ReadTables<'_>) -> Result<ComponentEntry, String> {
    let prop =
        |id: &str| tables.props.get(id).cloned().ok_or_else(|| format!("prop id {id} is not in the `props` table"));

    let Some(serde_json::Value::Array(set_ids)) = component.get("props") else {
        return Err("`props` holds something other than a list of prop set ids".to_owned());
    };
    let mut props = serde_json::Map::new();
    for set_id in set_ids {
        let set_id = set_id.as_str().ok_or_else(|| format!("`props` holds {set_id} where a prop set id belongs"))?;
        let set = tables
            .prop_sets
            .get(set_id)
            .ok_or_else(|| format!("prop set id {set_id} is not in the `propSets` table"))?;
        for (name, id) in set {
            if props.insert(name.clone(), prop(id)?).is_some() {
                return Err(format!("prop `{name}` is in two of the component's prop sets"));
            }
        }
    }
    component["props"] = serde_json::Value::Object(props);

    if let Some(serde_json::Value::Object(notable)) = component.get_mut("notableInherited") {
        for value in notable.values_mut() {
            let id =
                value.as_str().ok_or_else(|| format!("`notableInherited` holds {value} where a prop id belongs"))?;
            *value = prop(id)?;
        }
    }

    if let Some(serde_json::Value::Array(layers)) = component.get_mut("inheritance") {
        for layer in layers {
            let Some(omitted) = layer.get_mut("omitted") else { continue };
            let id =
                omitted.as_str().ok_or_else(|| format!("`omitted` holds {omitted} where a name list id belongs"))?;
            let names = tables
                .name_lists
                .get(id)
                .ok_or_else(|| format!("name list id {id} is not in the `nameLists` table"))?;
            *omitted = serde_json::json!(names);
        }
    }
    serde_json::from_value(component).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
