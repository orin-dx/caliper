use miette::Result;

/// Build the JSON Schema for the caliper-docgen `ExtractionOutput` format.
/// Hand-maintained per ADR-0002-style precedent — see the drift-detection
/// test below for the guard against it silently falling out of sync with the
/// real `ComponentEntry`/`ExtractionStats`/`Diagnostic` structs.
fn schema_value() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "ExtractionOutput",
        "description": "Output schema for caliper-docgen component prop extraction",
        "type": "object",
        "required": ["schemaVersion", "components", "props", "propSets", "nameLists", "enums", "diagnostics", "stats"],
        "definitions": {
            "propId": {
                "description": "A prop's key in the top-level `props` table: the FNV-1a 64 hash of its compact JSON (ADR 0009)",
                "type": "string",
                "pattern": "^[0-9a-f]{16}$"
            },
            "propSetId": {
                "description": "A prop set's key in the top-level `propSets` table: the hash of its (name, prop id) entries (ADR 0018)",
                "type": "string",
                "pattern": "^[0-9a-f]{16}$"
            },
            "nameListId": {
                "description": "A list's key in the top-level `nameLists` table: the hash of its names, in order (ADR 0018)",
                "type": "string",
                "pattern": "^[0-9a-f]{16}$"
            },
            "componentExport": {
                "description": "One way a module exports a component: `Card` for `export function Card` and for `export { _Card as Card }`, `default` for `export default Menu`, and for a sub-component attached to an exported component the export it hangs from with the properties that reach it, so `Menu.Item = Item` beside `export default Menu` is `default` with members `[\"Item\"]`",
                "type": "object",
                "required": ["name", "members"],
                "properties": {
                    "name": { "type": "string" },
                    "members": { "type": "array", "items": { "type": "string" } }
                }
            },
            "opaqueReason": {
                "description": "Why a prop type is `opaque`: `type` of the `reason` in an `{ kind: \"opaque\" }` type, with the named fields of its variant",
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": { "type": "string", "enum": caliper_docgen_core::types::OpaqueReason::KINDS }
                }
            },
            "prop": {
                "type": "object",
                "required": ["name", "required", "type"],
                "properties": {
                    "name": { "type": "string" },
                    "required": { "type": "boolean" },
                    "type": { "type": "object" },
                    "description": { "type": ["string", "null"] },
                    "defaultValue": { "type": ["object", "null"] },
                    "tags": { "type": "object" },
                    "parent": { "type": ["object", "null"] },
                    "declarations": { "type": "array" }
                }
            }
        },
        "properties": {
            "schemaVersion": {
                "description": "The shape of this JSON (ADR 0017). A reader checks it equals the version it was written for",
                "type": "integer",
                "const": caliper_docgen_core::types::SCHEMA_VERSION
            },
            "components": {
                "type": "object",
                "additionalProperties": {
                    "type": "object",
                    "required": ["displayName", "filePath", "props"],
                    "properties": {
                        "displayName": { "type": "string" },
                        "filePath": { "type": "string" },
                        "description": { "type": ["string", "null"] },
                        "props": {
                            "description": "The prop sets that together hold the component's props; a prop name is in one",
                            "type": "array",
                            "items": { "$ref": "#/definitions/propSetId" }
                        },
                        "inheritance": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "typeName": { "type": "string" },
                                    "fileName": { "type": "string" },
                                    "omitted": { "$ref": "#/definitions/nameListId" },
                                    "htmlElement": { "type": ["string", "null"] },
                                    "totalProps": { "type": "integer" },
                                    "members": { "$ref": "#/definitions/nameListId" }
                                }
                            }
                        },
                        "notableInherited": {
                            "description": "Prop name to prop id",
                            "type": "object",
                            "additionalProperties": { "$ref": "#/definitions/propId" }
                        },
                        "discriminantProp": { "type": ["string", "null"] },
                        "composes": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "tags": { "type": "object" },
                        "methods": { "type": "array" },
                        "detectedBy": {
                            "type": "string",
                            "enum": caliper_docgen_core::types::DetectionRule::ALL.map(|rule| rule.name())
                        },
                        "exported": {
                            "description": "Whether its module exports it, directly or as a sub-component attached to an exported component",
                            "type": "boolean"
                        },
                        "exports": {
                            "description": "How its module exports it, in source order (ADR 0027). `exported` is whether there are any",
                            "type": "array",
                            "items": { "$ref": "#/definitions/componentExport" }
                        }
                    }
                }
            },
            "props": {
                "description": "Every distinct prop the components refer to, by id",
                "type": "object",
                "additionalProperties": { "$ref": "#/definitions/prop" }
            },
            "propSets": {
                "description": "Every prop set the components refer to, by id: prop name to prop id",
                "type": "object",
                "additionalProperties": {
                    "type": "object",
                    "additionalProperties": { "$ref": "#/definitions/propId" }
                }
            },
            "nameLists": {
                "description": "Every list of names the components and diagnostics refer to, by id",
                "type": "object",
                "additionalProperties": { "type": "array", "items": { "type": "string" } }
            },
            "enums": { "type": "object" },
            "diagnostics": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["severity", "message", "code"],
                    "properties": {
                        "severity": { "type": "string", "enum": ["error", "warning", "info"] },
                        "message": { "type": "string" },
                        "file": { "type": ["string", "null"] },
                        "line": { "type": ["integer", "null"] },
                        "column": { "type": ["integer", "null"] },
                        "help": { "type": ["string", "null"] },
                        "code": { "type": "string" },
                        "affects": {
                            "description": "The components whose resolution reported this diagnostic: a list id in `nameLists`",
                            "$ref": "#/definitions/nameListId"
                        }
                    }
                }
            },
            "stats": {
                "type": "object",
                "required": ["componentsExtracted", "filesParsed", "durationMs"],
                "properties": {
                    "componentsExtracted": { "type": "integer" },
                    "componentsSkipped": { "type": "integer" },
                    "filesParsed": { "type": "integer" },
                    "dtsFilesParsed": { "type": "integer" },
                    "dtsCacheHits": { "type": "integer" },
                    "durationMs": { "type": "integer" },
                    "tier1Count": { "type": "integer" },
                    "tier3Count": { "type": "integer" },
                    "opaqueCount": { "type": "integer" }
                }
            }
        }
    })
}

/// Output JSON schema for the caliper-docgen ExtractionOutput format.
pub fn cmd_schema() -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&schema_value()).unwrap_or_default());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_repo_file(relative: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    #[test]
    fn detected_by_enum_lists_every_rust_variant_in_order() {
        let schema = schema_value();
        let listed: Vec<&str> = schema["properties"]["components"]["additionalProperties"]["properties"]["detectedBy"]
            ["enum"]
            .as_array()
            .expect("detectedBy enum is an array")
            .iter()
            .map(|value| value.as_str().expect("variant names are strings"))
            .collect();
        assert_eq!(
            listed,
            [
                "fcAnnotation",
                "forwardRef",
                "forwardRefExoticDecl",
                "hocWrapper",
                "classComponent",
                "functionDeclaration",
                "anonymousDefaultExport",
                "attachedSubComponent",
                "typedFunctionExpression",
                "wrapperTypeArgument",
                "importedMemberAlias",
                "memoExoticDecl",
                "taggedFactory",
                "factoryResult",
            ]
        );
    }

    #[test]
    fn typescript_detection_rule_union_matches_the_rust_enum() {
        let declarations = read_repo_file("packages/napi/index.d.ts");
        let union = declarations
            .split("export type DetectionRule =")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .expect("index.d.ts declares DetectionRule");
        let ts_names: Vec<&str> =
            union.lines().filter_map(|line| line.trim().strip_prefix("| '")?.strip_suffix('\'')).collect();
        let rust_names: Vec<String> =
            caliper_docgen_core::types::DetectionRule::ALL.iter().map(|rule| rule.name()).collect();
        assert_eq!(ts_names, rust_names.iter().map(String::as_str).collect::<Vec<_>>());
    }

    #[test]
    fn typescript_opaque_reason_union_matches_the_rust_enum() {
        let declarations = read_repo_file("packages/napi/index.d.ts");
        let union = declarations
            .split("export type OpaqueReason =")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .expect("index.d.ts declares OpaqueReason");
        let ts_names: Vec<&str> =
            union.lines().filter_map(|line| line.trim().strip_prefix("| { type: '")?.split('\'').next()).collect();
        assert_eq!(ts_names, caliper_docgen_core::types::OpaqueReason::KINDS);
    }

    #[test]
    fn schema_opaque_reason_enum_lists_every_rust_variant() {
        let schema = schema_value();
        let listed: Vec<&str> = schema["definitions"]["opaqueReason"]["properties"]["type"]["enum"]
            .as_array()
            .expect("opaqueReason type enum is an array")
            .iter()
            .map(|value| value.as_str().expect("names are strings"))
            .collect();
        assert_eq!(listed, caliper_docgen_core::types::OpaqueReason::KINDS);
    }

    #[test]
    fn every_reader_declares_the_schema_version_rust_writes() {
        let version = caliper_docgen_core::types::SCHEMA_VERSION;
        let python = format!("SUPPORTED_SCHEMA_VERSION = {version}");
        let typescript = format!("export const {python}");
        let readers = [
            ("scripts/schema_version.py", &python),
            ("apps/oracle/src/schema-version.ts", &typescript),
            ("packages/storybook/src/schema.ts", &typescript),
            ("packages/vite-plugin/src/schema.ts", &typescript),
        ];
        for (path, declaration) in readers {
            assert!(
                read_repo_file(path).lines().any(|line| line == declaration),
                "{path} must declare `{declaration}`"
            );
        }
    }

    #[test]
    fn test_cmd_schema_valid_json() {
        assert!(cmd_schema().is_ok());
    }

    #[test]
    fn schema_covers_every_field_name_the_real_output_serializes() {
        use caliper_docgen_core::types::{
            ComponentEntry, DefaultValue, Diagnostic, DiagnosticCode, DiagnosticSeverity, ExtractionOutput,
            ExtractionStats, InheritedLayer, ParsedProp, PropParent, PropType,
        };
        use std::collections::BTreeMap;

        let mut props = BTreeMap::new();
        props.insert(
            "variant".to_string(),
            std::sync::Arc::new(ParsedProp::new(
                "variant".into(),
                PropType::String,
                true,
                Some(DefaultValue { value: "\"a\"".into(), computed: false }),
                "desc".into(),
                BTreeMap::from([("since".to_string(), "1.0".to_string())]),
                Some(PropParent { name: "ButtonProps".into(), file_name: "Button.tsx".into() }),
                vec![PropParent { name: "ButtonProps".into(), file_name: "Button.tsx".into() }],
            )),
        );

        let mut components = BTreeMap::new();
        components.insert(
            "Button".to_string(),
            ComponentEntry {
                display_name: "Button".into(),
                file_path: "src/Button.tsx".into(),
                description: "A button".into(),
                props,
                inheritance: vec![InheritedLayer {
                    type_name: "ButtonHTMLAttributes".into(),
                    file_name: "react.d.ts".into(),
                    omitted: vec!["ref".into()],
                    html_element: Some("button".into()),
                    total_props: 3,
                    members: vec![],
                }],
                notable_inherited: BTreeMap::from([(
                    "onClick".to_string(),
                    std::sync::Arc::new(ParsedProp::new(
                        "onClick".into(),
                        PropType::Named { name: "MouseEventHandler".into(), args: vec![] },
                        false,
                        None,
                        String::new(),
                        BTreeMap::new(),
                        None,
                        vec![],
                    )),
                )]),
                discriminant_prop: Some("variant".into()),
                composes: vec!["SomeUnresolved".into()],
                tags: BTreeMap::from([("deprecated".to_string(), String::new())]),
                methods: vec![],
                detected_by: caliper_docgen_core::types::DetectionRule::FunctionDeclaration,
                exported: true,
                exports: vec![
                    caliper_docgen_core::types::ComponentExport::named("default"),
                    caliper_docgen_core::types::ComponentExport::named("default").member("Item"),
                ],
            },
        );

        let output = ExtractionOutput {
            components,
            enums: BTreeMap::new(),
            diagnostics: vec![Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "msg".into(),
                file: Some("Button.tsx".into()),
                line: Some(10),
                column: Some(4),
                help: Some("try this".into()),
                code: DiagnosticCode::OpaqueType,
                affects: Vec::new(),
            }],
            stats: ExtractionStats {
                components_extracted: 1,
                components_skipped: 1,
                files_parsed: 1,
                dts_files_parsed: 1,
                dts_cache_hits: 3,
                duration_ms: 5,
                tier1_count: 1,
                tier3_count: 1,
                opaque_count: 1,
            },
        };

        let value = serde_json::to_value(&output).expect("ExtractionOutput must serialize");
        let mut real_fields: Vec<String> = Vec::new();
        for key in ["components", "props", "diagnostics", "stats"] {
            assert!(value.get(key).is_some(), "fixture is missing top-level key {key}");
        }
        real_fields.extend(value.as_object().expect("output is an object").keys().cloned());
        if let Some(obj) = value["components"]["Button"].as_object() {
            real_fields.extend(obj.keys().cloned());
        }
        let set_id = value["components"]["Button"]["props"][0].as_str().expect("props list prop set ids");
        let variant_id = value["propSets"][set_id]["variant"].as_str().expect("prop sets hold prop ids");
        if let Some(obj) = value["props"][variant_id].as_object() {
            real_fields.extend(obj.keys().cloned());
        }
        if let Some(obj) = value["components"]["Button"]["inheritance"][0].as_object() {
            real_fields.extend(obj.keys().cloned());
        }
        if let Some(obj) = value["diagnostics"][0].as_object() {
            real_fields.extend(obj.keys().cloned());
        }
        if let Some(obj) = value["stats"].as_object() {
            real_fields.extend(obj.keys().cloned());
        }

        let schema_str = serde_json::to_string(&schema_value()).expect("schema must serialize");
        let missing: Vec<&String> = real_fields.iter().filter(|f| !schema_str.contains(f.as_str())).collect();
        assert!(missing.is_empty(), "schema.rs is missing field(s) present in real serialized output: {missing:?}");

        // ── SPEC-SERIALIZATION-001 AC-7: the fixture must populate every
        // optional/collection field at these 5 sampled paths with a real,
        // non-default value — a runtime assertion, not just a substring
        // presence check, so a field silently missing from schema_value()
        // shows up as an actual non-default value the diff above can catch.
        let button = &value["components"]["Button"];
        assert_ne!(button["notableInherited"], serde_json::json!({}), "notableInherited must be non-empty");
        assert_ne!(button["inheritance"][0]["omitted"], serde_json::json!([]), "omitted must be non-empty");
        assert!(!button["discriminantProp"].is_null(), "discriminantProp must be non-null");
        assert_ne!(button["composes"], serde_json::json!([]), "composes must be non-empty");

        let schema = schema_value();
        let component_schema = &schema["properties"]["components"]["additionalProperties"]["properties"];
        assert_eq!(
            component_schema["exports"]["items"],
            serde_json::json!({ "$ref": "#/definitions/componentExport" })
        );
        let export_fields: Vec<&String> =
            button["exports"][1].as_object().expect("an export is an object").keys().collect();
        let described: Vec<&String> = schema["definitions"]["componentExport"]["properties"]
            .as_object()
            .expect("componentExport lists its properties")
            .keys()
            .collect();
        assert_eq!(export_fields, described);
        assert_ne!(button["tags"], serde_json::json!({}), "component tags must be non-empty");

        let variant = &value["props"][variant_id];
        assert_ne!(variant["tags"], serde_json::json!({}), "prop tags must be non-empty");
        assert_ne!(variant["declarations"], serde_json::json!([]), "declarations must be non-empty");
        assert!(!variant["parent"].is_null(), "parent must be non-null");
        assert!(!variant["defaultValue"].is_null(), "defaultValue must be non-null");

        let diag = &value["diagnostics"][0];
        assert!(!diag["line"].is_null(), "diagnostic line must be non-null");
        assert!(!diag["column"].is_null(), "diagnostic column must be non-null");
        assert!(!diag["help"].is_null(), "diagnostic help must be non-null");

        let stats = value["stats"].as_object().expect("stats must be an object");
        for (key, val) in stats {
            assert_ne!(val, &serde_json::json!(0), "ExtractionStats field '{key}' must be non-zero in the fixture");
        }
    }
}
