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
        "required": ["schemaVersion", "components", "props", "enums", "diagnostics", "stats"],
        "definitions": {
            "propId": {
                "description": "A prop's key in the top-level `props` table: the FNV-1a 64 hash of its compact JSON (ADR 0009)",
                "type": "string",
                "pattern": "^[0-9a-f]{16}$"
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
                            "description": "Prop name to prop id",
                            "type": "object",
                            "additionalProperties": { "$ref": "#/definitions/propId" }
                        },
                        "inheritance": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "typeName": { "type": "string" },
                                    "fileName": { "type": "string" },
                                    "omitted": { "type": "array", "items": { "type": "string" } },
                                    "htmlElement": { "type": ["string", "null"] },
                                    "totalProps": { "type": "integer" }
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
                            "enum": [
                                "fcAnnotation", "forwardRef", "forwardRefExoticDecl", "hocWrapper", "classComponent",
                                "functionDeclaration", "anonymousDefaultExport", "attachedSubComponent",
                                "typedFunctionExpression", "wrapperTypeArgument"
                            ]
                        },
                        "exported": {
                            "description": "Whether its module exports it, directly or as a sub-component attached to an exported component",
                            "type": "boolean"
                        }
                    }
                }
            },
            "props": {
                "description": "Every distinct prop the components refer to, by id",
                "type": "object",
                "additionalProperties": { "$ref": "#/definitions/prop" }
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
                        "affects": { "type": "array", "items": { "type": "string" } }
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
        let variant_id = value["components"]["Button"]["props"]["variant"].as_str().expect("props hold ids");
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
