//! TOON (Token-Optimized Object Notation) renderer for Component Extraction metadata.
//!
//! Inspired by `michi` (TOON lists and token-optimized agent formatting).
//! Formats component props and metadata into a compact, low-token text format
//! ideal for LLM prompts, agent context windows, and terminal summaries.

use std::fmt::Write;

use crate::types::{ComponentEntry, ExtractionOutput, PropType};

/// Render a single component's prop table into TOON format.
pub fn render_component_toon(entry: &ComponentEntry) -> String {
    let mut out = String::new();
    writeln!(out, "component:{}:", entry.display_name).ok();
    writeln!(out, "  filePath: {}", entry.file_path).ok();
    if !entry.description.trim().is_empty() {
        writeln!(out, "  description: {}", entry.description.replace('\n', " ")).ok();
    }

    if !entry.props.is_empty() {
        writeln!(out, "  props[{}] {{name,required,type,default,description}}:", entry.props.len()).ok();
        for (prop_name, prop) in &entry.props {
            let req_str = if prop.required { "true" } else { "false" };
            let type_str = format_type_compact(&prop.prop_type);
            let default_str = prop.default_value.as_ref().map(|d| d.value.as_str()).unwrap_or("");
            let desc_str = prop.description.replace('\n', " ");

            // Format as CSV line: escape strings containing commas or quotes if needed
            writeln!(
                out,
                "    {},{},{},{},{}",
                escape_toon_val(prop_name),
                req_str,
                escape_toon_val(&type_str),
                escape_toon_val(default_str),
                escape_toon_val(&desc_str)
            )
            .ok();
        }
    }

    if !entry.inheritance.is_empty() {
        writeln!(out, "  inheritance[{}] {{typeName,htmlElement,totalProps}}:", entry.inheritance.len()).ok();
        for layer in &entry.inheritance {
            let elem_str = layer.html_element.as_deref().unwrap_or("none");
            writeln!(
                out,
                "    {},{},{}",
                escape_toon_val(&layer.type_name),
                escape_toon_val(elem_str),
                layer.total_props
            )
            .ok();
        }
    }

    out
}

/// Render an entire [`ExtractionOutput`] into TOON format.
pub fn render_output_toon(output: &ExtractionOutput) -> String {
    let mut out = String::new();
    if !output.components.is_empty() {
        writeln!(out, "docgenComponents[{}]:", output.components.len()).ok();
        for entry in output.components.values() {
            let comp_toon = render_component_toon(entry);
            for line in comp_toon.lines() {
                writeln!(out, "  {line}").ok();
            }
        }
    }

    if !output.diagnostics.is_empty() {
        writeln!(out, "diagnostics[{}]:", output.diagnostics.len()).ok();
        for diag in &output.diagnostics {
            let file_str = diag.file.as_deref().unwrap_or("unknown");
            writeln!(out, "  [{:?}]: {} ({})", diag.severity, diag.message, file_str).ok();
            if let Some(help) = &diag.help {
                writeln!(out, "    help: {help}").ok();
            }
            if !diag.affects.is_empty() {
                writeln!(out, "    affects: {}", diag.affects.join(", ")).ok();
            }
        }
    }

    writeln!(
        out,
        "stats: extracted={}, parsed={}, cacheHits={}, durationMs={}",
        output.stats.components_extracted,
        output.stats.files_parsed,
        output.stats.dts_cache_hits,
        output.stats.duration_ms
    )
    .ok();

    out
}

/// Truncate `parts` to `limit` items, appending a `"...(+N)"` marker for the
/// remainder instead of silently dropping them. Shared by every
/// `format_type_compact` branch that renders a bounded member list.
fn truncate_with_indicator(parts: &[String], limit: usize, sep: &str) -> String {
    if parts.len() <= limit {
        return parts.join(sep);
    }
    let mut shown: Vec<String> = parts[..limit].to_vec();
    shown.push(format!("...(+{})", parts.len() - limit));
    shown.join(sep)
}

/// Format a [`PropType`] as a compact string representation for TOON.
fn format_type_compact(prop_type: &PropType) -> String {
    match prop_type {
        PropType::String => "string".into(),
        PropType::Number => "number".into(),
        PropType::Boolean => "boolean".into(),
        PropType::Null => "null".into(),
        PropType::Undefined => "undefined".into(),
        PropType::Any => "any".into(),
        PropType::Never => "never".into(),
        PropType::Unknown => "unknown".into(),
        PropType::Void => "void".into(),
        PropType::StringLiteral(s) => format!("\"{s}\""),
        PropType::NumberLiteral(n) => n.to_string(),
        PropType::BoolLiteral(b) => b.to_string(),
        PropType::LiteralUnion { members, .. } => truncate_with_indicator(members, 6, "|"),
        // Parameter types only: the names cost tokens and say little an agent needs. An agent does need to know which
        // arguments it may leave out (`T?`) and which repeat (`...T[]`).
        PropType::Function { params, return_type, .. } => {
            let params: Vec<String> = params
                .iter()
                .map(|p| {
                    let ty = format_type_compact(&p.param_type);
                    match (p.rest, p.optional) {
                        (true, _) => format!("...{ty}"),
                        (false, true) => format!("{ty}?"),
                        (false, false) => ty,
                    }
                })
                .collect();
            format!("({}) => {}", params.join(", "), format_type_compact(return_type))
        }
        PropType::ReactNode => "ReactNode".into(),
        PropType::CssProperties => "CSSProperties".into(),
        PropType::ElementType => "ElementType".into(),
        PropType::Ref { element } => format!("Ref<{}>", element.as_deref().unwrap_or("unknown")),
        PropType::Object(_) => prop_type.raw_string(),
        PropType::Array(element_type) => format!("Array<{}>", format_type_compact(element_type)),
        PropType::Tuple(_) => prop_type.raw_string(),
        PropType::Union(members) => {
            let formatted: Vec<String> = members.iter().map(format_type_compact).collect();
            truncate_with_indicator(&formatted, 4, "|")
        }
        PropType::Intersection(members) => {
            let formatted: Vec<String> = members.iter().map(format_type_compact).collect();
            truncate_with_indicator(&formatted, 4, "&")
        }
        PropType::HtmlAttributes { element, .. } => format!("HTMLAttributes<{element}>"),
        PropType::Named { name, args } => {
            if args.is_empty() {
                name.to_string()
            } else {
                let formatted_args: Vec<String> = args.iter().map(format_type_compact).collect();
                format!("{name}<{}>", formatted_args.join(", "))
            }
        }
        PropType::SxProps => "SxProps".into(),
        PropType::Opaque(detail) => format!("opaque({})", detail.raw()),
    }
}

/// Escape a TOON field value if it contains commas, newlines, or quotes.
fn escape_toon_val(val: &str) -> String {
    if val.contains(',') || val.contains('\n') || val.contains('"') {
        format!("\"{}\"", val.replace('"', "\\\"").replace('\n', " "))
    } else {
        val.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ComponentEntry, ParsedProp};
    use std::collections::BTreeMap;

    #[test]
    fn test_render_component_toon() {
        let mut props = BTreeMap::new();
        props.insert(
            "variant".to_string(),
            std::sync::Arc::new(ParsedProp::new(
                "variant".into(),
                PropType::LiteralUnion { members: vec!["primary".into(), "secondary".into()], has_default: true },
                false,
                Some(crate::types::DefaultValue { value: "\"primary\"".into(), computed: false }),
                "Visual variant".into(),
                Default::default(),
                None,
                vec![],
            )),
        );

        let entry = ComponentEntry {
            display_name: "Button".into(),
            file_path: "src/Button.tsx".into(),
            props,
            description: "Standard button component".into(),
            inheritance: vec![],
            notable_inherited: Default::default(),
            discriminant_prop: None,
            composes: vec![],
            tags: Default::default(),
            methods: vec![],
            detected_by: crate::types::DetectionRule::FunctionDeclaration,
            exported: true,
        };

        let toon = render_component_toon(&entry);
        assert!(toon.contains("component:Button:"));
        assert!(toon.contains("filePath: src/Button.tsx"));
        assert!(toon.contains("props[1] {name,required,type,default,description}:"));
        assert!(toon.contains("variant,false,primary|secondary,"));
        assert!(toon.contains("Visual variant"));
    }

    #[test]
    fn test_render_output_toon_with_diagnostics_and_stats() {
        let mut output = ExtractionOutput {
            components: Default::default(),
            enums: Default::default(),
            diagnostics: vec![],
            stats: Default::default(),
        };
        output.stats.components_extracted = 1;
        output.stats.files_parsed = 2;
        output.stats.dts_cache_hits = 1;
        output.stats.duration_ms = 42;

        output.diagnostics.push(crate::types::Diagnostic {
            severity: crate::types::DiagnosticSeverity::Warning,
            message: "Missing export".into(),
            file: Some("src/Card.tsx".into()),
            line: None,
            column: None,
            code: crate::types::DiagnosticCode::UnresolvableImport,
            help: Some("Check import path".into()),
            affects: Vec::new(),
        });

        let toon = render_output_toon(&output);
        assert!(toon.contains("diagnostics[1]:"));
        assert!(toon.contains("Missing export (src/Card.tsx)"));
        assert!(toon.contains("help: Check import path"));
        assert!(toon.contains("stats: extracted=1, parsed=2, cacheHits=1, durationMs=42"));
    }

    #[test]
    fn test_format_type_compact_complex_types() {
        assert_eq!(format_type_compact(&PropType::Array(Box::new(PropType::String))), "Array<string>");
        assert_eq!(
            format_type_compact(&PropType::Ref { element: Some("HTMLButtonElement".into()) }),
            "Ref<HTMLButtonElement>"
        );
        let on_select = PropType::Function {
            type_params: vec![],
            params: vec![crate::types::FunctionParam::new(
                Some("event".into()),
                PropType::Named { name: "MouseEvent".into(), args: vec![] },
            )],
            return_type: Box::new(PropType::Boolean),
        };
        assert_eq!(format_type_compact(&on_select), "(MouseEvent) => boolean");
        let with_flags = PropType::Function {
            type_params: vec![],
            params: vec![
                crate::types::FunctionParam::new(Some("a".into()), PropType::String),
                crate::types::FunctionParam {
                    optional: true,
                    ..crate::types::FunctionParam::new(Some("b".into()), PropType::Number)
                },
                crate::types::FunctionParam {
                    rest: true,
                    ..crate::types::FunctionParam::new(
                        Some("rest".into()),
                        PropType::Array(Box::new(PropType::Boolean)),
                    )
                },
            ],
            return_type: Box::new(PropType::Void),
        };
        assert_eq!(format_type_compact(&with_flags), "(string, number?, ...Array<boolean>) => void");
        assert_eq!(
            format_type_compact(&PropType::HtmlAttributes { element: "button".into(), omitted: vec![] }),
            "HTMLAttributes<button>"
        );
        assert_eq!(format_type_compact(&PropType::SxProps), "SxProps");
        assert_eq!(
            format_type_compact(&crate::types::output::OpaqueDetail::new(
                "CustomType",
                crate::types::output::OpaqueReason::ConditionalType
            )),
            "opaque(CustomType)"
        );

        let large_union = PropType::LiteralUnion {
            members: vec![
                "a".into(),
                "b".into(),
                "c".into(),
                "d".into(),
                "e".into(),
                "f".into(),
                "g".into(),
                "h".into(),
            ],
            has_default: false,
        };
        assert_eq!(format_type_compact(&large_union), "a|b|c|d|e|f|...(+2)");
    }

    #[test]
    fn test_format_type_compact_union_truncates_with_indicator() {
        let union = PropType::Union(vec![
            PropType::StringLiteral("a".into()),
            PropType::StringLiteral("b".into()),
            PropType::StringLiteral("c".into()),
            PropType::StringLiteral("d".into()),
            PropType::StringLiteral("e".into()),
            PropType::StringLiteral("f".into()),
        ]);
        let out = format_type_compact(&union);
        assert!(out.contains("...(+2)"), "expected a truncation indicator for the 2 dropped members, got: {out}");
    }

    #[test]
    fn test_format_type_compact_intersection_truncates_with_indicator() {
        let intersection = PropType::Intersection(vec![
            PropType::Named { name: "A".into(), args: vec![] },
            PropType::Named { name: "B".into(), args: vec![] },
            PropType::Named { name: "C".into(), args: vec![] },
            PropType::Named { name: "D".into(), args: vec![] },
            PropType::Named { name: "E".into(), args: vec![] },
        ]);
        let out = format_type_compact(&intersection);
        assert!(out.contains("...(+1)"), "expected a truncation indicator for the 1 dropped member, got: {out}");
    }

    // ── SPEC-SERIALIZATION-001 AC-5: at-or-under-cap fixtures (no truncation
    // marker at all) and a multi-digit-N fixture, per variant, so the
    // "no format truncates silently" claim is falsifiable in both directions
    // and N isn't assumed single-digit.

    #[test]
    fn test_format_type_compact_union_at_cap_has_no_truncation_marker() {
        let union = PropType::Union(vec![
            PropType::StringLiteral("a".into()),
            PropType::StringLiteral("b".into()),
            PropType::StringLiteral("c".into()),
            PropType::StringLiteral("d".into()),
        ]);
        let out = format_type_compact(&union);
        assert!(!out.contains("...(+"), "at-cap union must not truncate, got: {out}");
    }

    #[test]
    fn test_format_type_compact_intersection_at_cap_has_no_truncation_marker() {
        let intersection = PropType::Intersection(vec![
            PropType::Named { name: "A".into(), args: vec![] },
            PropType::Named { name: "B".into(), args: vec![] },
            PropType::Named { name: "C".into(), args: vec![] },
            PropType::Named { name: "D".into(), args: vec![] },
        ]);
        let out = format_type_compact(&intersection);
        assert!(!out.contains("...(+"), "at-cap intersection must not truncate, got: {out}");
    }

    #[test]
    fn test_format_type_compact_literal_union_at_cap_has_no_truncation_marker() {
        let literal_union = PropType::LiteralUnion {
            members: vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into(), "f".into()],
            has_default: false,
        };
        let out = format_type_compact(&literal_union);
        assert!(!out.contains("...(+"), "at-cap literal union must not truncate, got: {out}");
    }

    #[test]
    fn test_format_type_compact_literal_union_multi_digit_truncation_count() {
        let members: Vec<String> = (0..20).map(|i| format!("v{i}")).collect();
        let literal_union = PropType::LiteralUnion { members, has_default: false };
        let out = format_type_compact(&literal_union);
        assert!(out.contains("...(+14)"), "expected a 2-digit truncation count (20 members, cap 6 -> +14), got: {out}");
    }
}
