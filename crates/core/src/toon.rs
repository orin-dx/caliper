//! TOON (Token-Optimized Object Notation) renderer for Component Extraction metadata.
//!
//! Inspired by `michi` (TOON lists and token-optimized agent formatting).
//! Formats component props and metadata into a compact, low-token text format
//! ideal for LLM prompts, agent context windows, and terminal summaries.
//!
//! # Lossy by design
//!
//! TOON is written for a reader that has a token budget, not for a program that reads it back, so it leaves out what
//! costs tokens and says little. It carries no `schemaVersion` and has no parser. Compared with the canonical JSON:
//!
//! - A literal union is never cut, nor is a union with a literal member (`'sm'|'md'|null`): an agent writes a call
//!   from those values and cannot guess the missing ones. A union of types only prints at most 8 members, an
//!   intersection at most 4 and a layer's `members` at most 24, each followed by `...(+N)` for the rest; the canonical
//!   JSON holds every member. A string literal is written in single quotes (`'primary'`), so a member can be told from
//!   a type name.
//! - A function type prints its parameters with their names (`(value: string, index?: number) => void`); an object or
//!   tuple type prints as its source text.
//! - A prop's provenance is its `source` only: the name of the type that declared it, not the file or every
//!   declaration. Descriptions lose their line breaks.
//! - `degradedBy` (a column present only when some prop has it) names why a prop's type is partly `opaque`; a name left
//!   as an unexpanded reference is not flagged there, and the diagnostics below say so.
//! - A prop's `@deprecated`, `@see` and `@since` tags are written in a `tags` column (present only when some prop has
//!   one) as `deprecated=use X;see=https://…;since=2.1`; its other tags, the component's `tags`, `composes`,
//!   `notableInherited` and the id tables of the canonical JSON are not written.
//! - A discriminated union names its discriminant prop in a `discriminant:` line. Its members' props are merged into
//!   one list, each prop's `source` names the member that declares it and each prop absent from some member is
//!   optional.

use std::fmt::Write;

use crate::types::{ComponentEntry, Diagnostic, DiagnosticSeverity, ExtractionOutput, PropType};

/// The most names of a collapsed layer's `members` one `inheritance` row lists.
const LAYER_MEMBERS_LIMIT: usize = 24;

/// The most members a union of types (no literal among them) prints. Real antd props reach 5 members at most in
/// 2 of 5049 cases; 8 leaves headroom, and the cut is marked.
const UNION_LIMIT: usize = 8;

/// The most members an intersection prints.
const INTERSECTION_LIMIT: usize = 4;

/// The tags of a prop that change how an agent uses it, in the order they are written.
const PROP_TAGS: [&str; 3] = ["deprecated", "see", "since"];

/// Render a single component's prop table into TOON format.
pub fn render_component_toon(entry: &ComponentEntry) -> String {
    let mut out = String::new();
    writeln!(out, "component:{}:", entry.display_name).ok();
    writeln!(out, "  filePath: {}", entry.file_path).ok();
    if !entry.description.trim().is_empty() {
        writeln!(out, "  description: {}", entry.description.replace('\n', " ")).ok();
    }

    if let Some(discriminant) = &entry.discriminant_prop {
        writeln!(out, "  discriminant: {discriminant}").ok();
    }

    if !entry.props.is_empty() {
        // A column exists only when some prop has something to put in it.
        let degraded = entry.props.values().any(|prop| !prop.degraded_by.is_empty());
        let degraded_header = if degraded { "degradedBy," } else { "" };
        let tagged = entry.props.values().any(|prop| !prop_tags_cell(prop).is_empty());
        let tags_header = if tagged { "tags," } else { "" };
        writeln!(
            out,
            "  props[{}] {{name,required,type,default,source,{degraded_header}{tags_header}description}}:",
            entry.props.len()
        )
        .ok();
        for (prop_name, prop) in &entry.props {
            let req_str = if prop.required { "true" } else { "false" };
            let type_str = format_type_compact(&prop.prop_type);
            let default_str = prop.default_value.as_ref().map(|d| d.value.as_str()).unwrap_or("");
            let source_str = prop.parent.as_ref().map(|parent| parent.name.as_str()).unwrap_or("");
            let desc_str = prop.description.replace('\n', " ");

            // Format as CSV line: escape strings containing commas or quotes if needed
            let degraded_cell =
                if degraded { format!("{},", escape_toon_val(&prop.degraded_by.join("|"))) } else { String::new() };
            let tags_cell = if tagged { format!("{},", escape_toon_val(&prop_tags_cell(prop))) } else { String::new() };
            writeln!(
                out,
                "    {},{},{},{},{},{}{}{}",
                escape_toon_val(prop_name),
                req_str,
                escape_toon_val(&type_str),
                escape_toon_val(default_str),
                escape_toon_val(source_str),
                degraded_cell,
                tags_cell,
                escape_toon_val(&desc_str)
            )
            .ok();
        }
    }

    if !entry.inheritance.is_empty() {
        // A collapsed layer lists the names of the props it declares; the column exists only when one does.
        let members = entry.inheritance.iter().any(|layer| !layer.members.is_empty());
        let members_header = if members { ",members" } else { "" };
        writeln!(
            out,
            "  inheritance[{}] {{typeName,htmlElement,totalProps{members_header}}}:",
            entry.inheritance.len()
        )
        .ok();
        for layer in &entry.inheritance {
            let elem_str = layer.html_element.as_deref().unwrap_or("none");
            let members_cell = if members {
                format!(",{}", escape_toon_val(&truncate_with_indicator(&layer.members, LAYER_MEMBERS_LIMIT, "|")))
            } else {
                String::new()
            };
            writeln!(
                out,
                "    {},{},{}{}",
                escape_toon_val(&layer.type_name),
                escape_toon_val(elem_str),
                layer.total_props,
                members_cell
            )
            .ok();
        }
    }

    out
}

/// One component as an agent reads it: its [`render_component_toon`] table, then the diagnostics that say its props may
/// be incomplete (warnings and errors; `info` says nothing about the props), each with its fix when it has one. This is
/// the slice a host asks for when it needs one component and not the output of a whole project.
pub fn render_component_slice_toon(entry: &ComponentEntry, diagnostics: &[Diagnostic]) -> String {
    let mut out = render_component_toon(entry);
    let incomplete: Vec<&Diagnostic> =
        diagnostics.iter().filter(|diag| diag.severity != DiagnosticSeverity::Info).collect();
    if !incomplete.is_empty() {
        writeln!(out, "  diagnostics[{}]:", incomplete.len()).ok();
        for diag in incomplete {
            writeln!(out, "    [{:?}]: {}", diag.severity, diag.message.replace('\n', " ")).ok();
            if let Some(help) = &diag.help {
                writeln!(out, "      help: {}", help.replace('\n', " ")).ok();
            }
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

/// A prop's `@deprecated`, `@see` and `@since` tags as `deprecated=use X;since=2.1`; a tag with no text is its name
/// alone. Empty when the prop has none.
fn prop_tags_cell(prop: &crate::types::ParsedProp) -> String {
    PROP_TAGS
        .iter()
        .filter_map(|name| {
            let text = prop.tags.get(*name)?.split_whitespace().collect::<Vec<_>>().join(" ");
            Some(if text.is_empty() { (*name).to_owned() } else { format!("{name}={text}") })
        })
        .collect::<Vec<_>>()
        .join(";")
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
        PropType::BigInt => "bigint".into(),
        PropType::StringLiteral(s) => quote_literal(s),
        PropType::NumberLiteral(n) => n.to_string(),
        PropType::BoolLiteral(b) => b.to_string(),
        // A bigint `10n`, `typeof brand`, an enum member `Size.Small` and a template `` `${number}px` `` print as
        // TypeScript writes them, which is short already.
        PropType::BigIntLiteral(_)
        | PropType::UniqueSymbol { .. }
        | PropType::EnumMember { .. }
        | PropType::TemplateLiteral(_) => prop_type.raw_string(),
        PropType::LiteralUnion { members, .. } => {
            // Every value: it is what an agent writes a valid call from.
            members.iter().map(|member| quote_literal(member)).collect::<Vec<_>>().join("|")
        }
        // The names say what each argument is for (`(value: string)` and not `(string)`), and the markers which
        // arguments may be left out (`name?: T`) and which repeat (`...name: T[]`). A destructured parameter has no
        // name and prints its type alone.
        PropType::Function { params, return_type, .. } => {
            let params: Vec<String> = params
                .iter()
                .map(|p| {
                    let ty = format_type_compact(&p.param_type);
                    let (rest, optional) = (if p.rest { "..." } else { "" }, if p.optional { "?" } else { "" });
                    match &p.name {
                        Some(name) => format!("{rest}{name}{optional}: {ty}"),
                        None => format!("{rest}{ty}{optional}"),
                    }
                })
                .collect();
            format!("({}) => {}", params.join(", "), format_type_compact(return_type))
        }
        PropType::ReactNode => "ReactNode".into(),
        PropType::CssProperties => "CSSProperties".into(),
        PropType::ElementType => "ElementType".into(),
        PropType::Ref { name, element } => format!("{name}<{}>", element.as_deref().unwrap_or("unknown")),
        PropType::Readonly(inner) => match &**inner {
            PropType::Array(element_type) => format!("ReadonlyArray<{}>", format_type_compact(element_type)),
            other => format!("readonly {}", format_type_compact(other)),
        },
        PropType::Object(_) | PropType::Callable { .. } => prop_type.raw_string(),
        PropType::Array(element_type) => format!("Array<{}>", format_type_compact(element_type)),
        PropType::Tuple(_) => prop_type.raw_string(),
        PropType::Union(members) => {
            let formatted: Vec<String> = members.iter().map(format_type_compact).collect();
            // A literal among the members is a value an agent may write: show every one. Only a union of types is cut.
            let has_literal = members
                .iter()
                .any(|member| member.literal_kind().is_some() || matches!(member, PropType::LiteralUnion { .. }));
            if has_literal { formatted.join("|") } else { truncate_with_indicator(&formatted, UNION_LIMIT, "|") }
        }
        PropType::Intersection(members) => {
            let formatted: Vec<String> = members.iter().map(format_type_compact).collect();
            truncate_with_indicator(&formatted, INTERSECTION_LIMIT, "&")
        }
        PropType::HtmlAttributes { element, .. } => format!("HTMLAttributes<{element}>"),
        // A recursive reference is the name of the type it is inside.
        PropType::Named { name, args } | PropType::Recursive { name, args } => {
            if args.is_empty() {
                name.to_string()
            } else {
                let formatted_args: Vec<String> = args.iter().map(format_type_compact).collect();
                format!("{name}<{}>", formatted_args.join(", "))
            }
        }
        PropType::SxProps => "SxProps".into(),
        PropType::Opaque(detail) => format!("opaque({})", detail.raw()),
        PropType::Truncated { text, .. } => format!("truncated({text})"),
    }
}

/// A string literal in single quotes, so `'primary'` is a value and `primary` a type name. A quote inside it is escaped.
fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
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
#[allow(clippy::disallowed_methods, reason = "tests build degraded types directly")]
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
            exports: Vec::new(),
        };

        let toon = render_component_toon(&entry);
        assert!(toon.contains("component:Button:"));
        assert!(toon.contains("filePath: src/Button.tsx"));
        assert!(toon.contains("props[1] {name,required,type,default,source,description}:"));
        assert!(toon.contains("variant,false,'primary'|'secondary',"));
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
    fn the_kinds_the_checker_states_print_as_typescript_writes_them() {
        use crate::types::{
            BigIntValue, CallSignature, EnumMemberValue, FunctionParam, SignatureKind, TemplatePart, TruncationLimit,
            TupleElement, TupleElementKind, TupleLabel,
        };
        let rest = PropType::Tuple(vec![
            TupleElement { label: TupleLabel::new("head"), ..TupleElement::required(PropType::String) },
            TupleElement {
                kind: TupleElementKind::Rest,
                label: TupleLabel::new("tail"),
                ..TupleElement::required(PropType::Number)
            },
        ]);
        assert_eq!(format_type_compact(&rest), "[head: string, ...tail: number[]]");
        let big = |v: &str| PropType::BigIntLiteral(BigIntValue::parse(v).unwrap());
        assert_eq!(format_type_compact(&PropType::Union(vec![big("10"), big("-1")])), "10n|-1n");
        assert_eq!(format_type_compact(&PropType::UniqueSymbol { name: "brand".into() }), "typeof brand");
        let template =
            PropType::TemplateLiteral(vec![TemplatePart::Type(PropType::Number), TemplatePart::Text("px".into())]);
        assert_eq!(format_type_compact(&template), "`${number}px`");
        let callable = PropType::Callable {
            signatures: vec![CallSignature {
                kind: SignatureKind::Construct,
                type_params: vec![],
                params: vec![FunctionParam::new(Some("value".into()), PropType::String)],
                return_type: PropType::Named { name: "Date".into(), args: vec![] },
            }],
            fields: vec![],
        };
        assert_eq!(format_type_compact(&callable), "{ new (value: string): Date }");
        let recursive = PropType::Recursive { name: "Tree".into(), args: vec![PropType::String] };
        assert_eq!(format_type_compact(&recursive), "Tree<string>");
        let truncated = PropType::Truncated { text: "SystemStyleObject".into(), limit: TruncationLimit::Depth };
        assert_eq!(format_type_compact(&truncated), "truncated(SystemStyleObject)");
        // A union of enum members is a union of values: never cut, however long.
        let members: Vec<PropType> = (0..10)
            .map(|i| PropType::EnumMember {
                enum_name: "Step".into(),
                member: format!("S{i}").into(),
                value: EnumMemberValue::Number(f64::from(i)),
            })
            .collect();
        let printed = format_type_compact(&PropType::Union(members));
        assert!(printed.ends_with("Step.S9") && !printed.contains("...(+"), "{printed}");
    }

    #[test]
    fn test_format_type_compact_complex_types() {
        assert_eq!(format_type_compact(&PropType::Array(Box::new(PropType::String))), "Array<string>");
        assert_eq!(
            format_type_compact(&PropType::Ref { name: "Ref".into(), element: Some("HTMLButtonElement".into()) }),
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
        assert_eq!(format_type_compact(&on_select), "(event: MouseEvent) => boolean");
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
        assert_eq!(format_type_compact(&with_flags), "(a: string, b?: number, ...rest: Array<boolean>) => void");
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
        assert_eq!(format_type_compact(&large_union), "'a'|'b'|'c'|'d'|'e'|'f'|'g'|'h'");
    }

    #[test]
    fn test_format_type_compact_union_truncates_with_indicator() {
        let union = PropType::Union(
            ('A'..='J').map(|name| PropType::Named { name: name.to_string().into(), args: vec![] }).collect(),
        );
        let out = format_type_compact(&union);
        assert_eq!(out, "A|B|C|D|E|F|G|H|...(+2)", "10 named types: 8 shown, the 2 dropped counted");
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
        let union = PropType::Union(
            ('A'..='H').map(|name| PropType::Named { name: name.to_string().into(), args: vec![] }).collect(),
        );
        let out = format_type_compact(&union);
        assert_eq!(out, "A|B|C|D|E|F|G|H", "at-cap union must not truncate");
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
    fn test_format_type_compact_union_multi_digit_truncation_count() {
        let union =
            PropType::Union((0..20).map(|i| PropType::Named { name: format!("T{i}").into(), args: vec![] }).collect());
        let out = format_type_compact(&union);
        assert!(
            out.ends_with("|...(+12)"),
            "expected a 2-digit truncation count (20 members, cap 8 -> +12), got: {out}"
        );
    }

    fn prop(name: &str, prop_type: PropType, parent: Option<&str>) -> std::sync::Arc<ParsedProp> {
        std::sync::Arc::new(ParsedProp::new(
            name.into(),
            prop_type,
            false,
            None,
            String::new(),
            Default::default(),
            parent.map(|parent| crate::types::PropParent { name: parent.into(), file_name: "src/types.ts".into() }),
            vec![],
        ))
    }

    fn entry_with(
        props: Vec<std::sync::Arc<ParsedProp>>,
        inheritance: Vec<crate::types::InheritedLayer>,
    ) -> ComponentEntry {
        ComponentEntry {
            display_name: "Widget".into(),
            file_path: "src/Widget.tsx".into(),
            props: props.into_iter().map(|prop| (prop.name.clone(), prop)).collect(),
            description: String::new(),
            inheritance,
            notable_inherited: Default::default(),
            discriminant_prop: None,
            composes: vec![],
            tags: Default::default(),
            methods: vec![],
            detected_by: crate::types::DetectionRule::FunctionDeclaration,
            exported: true,
            exports: Vec::new(),
        }
    }

    #[test]
    fn a_prop_row_names_the_type_that_declared_it() {
        let entry = entry_with(
            vec![prop("size", PropType::String, Some("WidgetProps")), prop("id", PropType::String, None)],
            vec![],
        );
        let toon = render_component_toon(&entry);
        assert!(toon.contains("props[2] {name,required,type,default,source,description}:\n"), "{toon}");
        assert!(toon.contains("\n    id,false,string,,,\n"), "no declaring type: an empty source\n{toon}");
        assert!(toon.contains("\n    size,false,string,,WidgetProps,\n"), "{toon}");
    }

    #[test]
    fn a_degraded_column_appears_only_when_a_prop_has_a_reason() {
        let opaque =
            crate::types::output::OpaqueDetail::new("keyof Foo", crate::types::output::OpaqueReason::MappedType);
        let entry = entry_with(
            vec![
                prop("status", opaque.clone(), Some("P")),
                prop("either", PropType::Union(vec![PropType::String, opaque]), Some("P")),
                prop("size", PropType::String, Some("P")),
            ],
            vec![],
        );
        let toon = render_component_toon(&entry);
        assert!(toon.contains("props[3] {name,required,type,default,source,degradedBy,description}:\n"), "{toon}");
        assert!(toon.contains("\n    status,false,opaque(keyof Foo),,P,mappedType,\n"), "{toon}");
        assert!(toon.contains("\n    either,false,string|opaque(keyof Foo),,P,mappedType,\n"), "{toon}");
        assert!(toon.contains("\n    size,false,string,,P,,\n"), "a prop without one has an empty cell\n{toon}");
    }

    #[test]
    fn literal_unions_and_literals_are_quoted_alike() {
        assert_eq!(format_type_compact(&PropType::StringLiteral("sm".into())), "'sm'");
        assert_eq!(
            format_type_compact(&PropType::LiteralUnion {
                members: vec!["sm".into(), "it's".into()],
                has_default: false
            }),
            "'sm'|'it\\'s'"
        );
        assert_eq!(
            format_type_compact(&PropType::Union(vec![
                PropType::StringLiteral("a".into()),
                PropType::Named { name: "a".into(), args: vec![] },
            ])),
            "'a'|a",
            "a value and a type of the same spelling can be told apart"
        );
    }

    #[test]
    fn a_collapsed_layer_lists_its_members_up_to_a_limit() {
        let layer = |name: &str, members: Vec<String>| crate::types::InheritedLayer {
            type_name: name.into(),
            file_name: "node_modules/x.d.ts".into(),
            omitted: vec![],
            html_element: None,
            total_props: members.len() as u32,
            members,
        };
        let many: Vec<String> = (0..30).map(|n| format!("p{n}")).collect();
        let entry = entry_with(
            vec![],
            vec![
                layer("HTMLAttributes", many),
                layer("Small", vec!["a".into(), "b".into()]),
                layer("Expanded", vec![]),
            ],
        );
        let toon = render_component_toon(&entry);
        assert!(toon.contains("inheritance[3] {typeName,htmlElement,totalProps,members}:\n"), "{toon}");
        let first = (0..24).map(|n| format!("p{n}")).collect::<Vec<_>>().join("|");
        assert!(toon.contains(&format!("\n    HTMLAttributes,none,30,{first}|...(+6)\n")), "{toon}");
        assert!(toon.contains("\n    Small,none,2,a|b\n"), "{toon}");
        assert!(toon.contains("\n    Expanded,none,0,\n"), "an expanded layer has an empty cell\n{toon}");
    }

    #[test]
    fn the_members_column_is_left_out_when_no_layer_collapses() {
        let layer = crate::types::InheritedLayer {
            type_name: "Base".into(),
            file_name: "src/base.ts".into(),
            omitted: vec![],
            html_element: Some("div".into()),
            total_props: 3,
            members: vec![],
        };
        let toon = render_component_toon(&entry_with(vec![], vec![layer]));
        assert!(toon.contains("inheritance[1] {typeName,htmlElement,totalProps}:\n    Base,div,3\n"), "{toon}");
    }

    fn diagnostic(severity: crate::types::DiagnosticSeverity, message: &str, help: Option<&str>) -> Diagnostic {
        Diagnostic {
            severity,
            message: message.into(),
            file: None,
            line: None,
            column: None,
            code: crate::types::DiagnosticCode::UnresolvableImport,
            help: help.map(str::to_owned),
            affects: Vec::new(),
        }
    }

    #[test]
    fn a_slice_is_the_component_and_the_diagnostics_that_say_its_props_may_be_incomplete() {
        let entry = entry_with(vec![prop("size", PropType::String, Some("P"))], vec![]);
        let diagnostics = [
            diagnostic(DiagnosticSeverity::Warning, "Cannot resolve type 'X'", Some("Install the package.")),
            diagnostic(DiagnosticSeverity::Info, "typeof 'Y' could not be evaluated", None),
            diagnostic(DiagnosticSeverity::Error, "no props\ntype", None),
        ];
        let toon = render_component_slice_toon(&entry, &diagnostics);
        assert!(toon.starts_with(&render_component_toon(&entry)), "{toon}");
        assert!(
            toon.ends_with(
                "  diagnostics[2]:\n    [Warning]: Cannot resolve type 'X'\n      help: Install the package.\n    [Error]: no props type\n"
            ),
            "{toon}"
        );
        assert!(!toon.contains("typeof 'Y'"), "info is left out: {toon}");
    }

    #[test]
    fn a_slice_without_warnings_is_the_component_alone() {
        let entry = entry_with(vec![prop("size", PropType::String, None)], vec![]);
        let diagnostics = [diagnostic(DiagnosticSeverity::Info, "note", None)];
        assert_eq!(render_component_slice_toon(&entry, &diagnostics), render_component_toon(&entry));
    }

    fn tagged_prop(name: &str, prop_type: PropType, tags: &[(&str, &str)]) -> std::sync::Arc<ParsedProp> {
        std::sync::Arc::new(ParsedProp::new(
            name.into(),
            prop_type,
            false,
            None,
            "Doc".into(),
            tags.iter().map(|(tag, text)| ((*tag).to_owned(), (*text).to_owned())).collect(),
            None,
            vec![],
        ))
    }

    #[test]
    fn a_shadcn_like_variant_and_size_list_every_value() {
        let variant = PropType::LiteralUnion {
            members: ["default", "destructive", "outline", "secondary", "ghost", "link", "success", "warning"]
                .map(String::from)
                .to_vec(),
            has_default: true,
        };
        let size = PropType::Union(
            ["default", "xs", "sm", "lg", "icon", "icon-xs", "icon-sm", "icon-lg"]
                .map(|value| PropType::StringLiteral(value.into()))
                .into_iter()
                .chain([PropType::Null])
                .collect(),
        );
        let toon =
            render_component_toon(&entry_with(vec![prop("variant", variant, None), prop("size", size, None)], vec![]));
        assert!(
            toon.contains("\n    size,false,'default'|'xs'|'sm'|'lg'|'icon'|'icon-xs'|'icon-sm'|'icon-lg'|null,,,\n"),
            "{toon}"
        );
        assert!(
            toon.contains(
                "\n    variant,false,'default'|'destructive'|'outline'|'secondary'|'ghost'|'link'|'success'|'warning',,,\n"
            ),
            "{toon}"
        );
        assert!(!toon.contains("...(+"), "{toon}");
    }

    #[test]
    fn a_function_prop_keeps_its_parameter_names() {
        let on_change = PropType::Function {
            type_params: vec![],
            params: vec![
                crate::types::FunctionParam::new(Some("value".into()), PropType::String),
                crate::types::FunctionParam {
                    optional: true,
                    ..crate::types::FunctionParam::new(Some("index".into()), PropType::Number)
                },
                crate::types::FunctionParam::new(None, PropType::Boolean),
            ],
            return_type: Box::new(PropType::Void),
        };
        let toon = render_component_toon(&entry_with(vec![prop("onChange", on_change, None)], vec![]));
        assert!(
            toon.contains("\n    onChange,false,\"(value: string, index?: number, boolean) => void\",,,\n"),
            "{toon}"
        );
    }

    #[test]
    fn deprecated_see_and_since_tags_get_a_column_that_only_tagged_components_have() {
        let tagged = tagged_prop(
            "old",
            PropType::String,
            &[("deprecated", "use `next`"), ("see", "https://x.dev/a,b"), ("since", "2.1"), ("internal", "")],
        );
        let bare_deprecated = tagged_prop("older", PropType::String, &[("deprecated", "")]);
        let plain = prop("plain", PropType::String, None);
        let toon = render_component_toon(&entry_with(vec![tagged, bare_deprecated, plain], vec![]));
        assert!(toon.contains("props[3] {name,required,type,default,source,tags,description}:\n"), "{toon}");
        assert!(
            toon.contains("\n    old,false,string,,,\"deprecated=use `next`;see=https://x.dev/a,b;since=2.1\",Doc\n"),
            "{toon}"
        );
        assert!(toon.contains("\n    older,false,string,,,deprecated,Doc\n"), "{toon}");
        assert!(toon.contains("\n    plain,false,string,,,,\n"), "{toon}");

        let untagged = render_component_toon(&entry_with(vec![prop("plain", PropType::String, None)], vec![]));
        assert!(untagged.contains("props[1] {name,required,type,default,source,description}:\n"), "{untagged}");
    }

    #[test]
    fn a_discriminated_union_names_its_discriminant() {
        let mut entry = entry_with(
            vec![
                prop("a", PropType::String, Some("A")),
                prop("b", PropType::Number, Some("B")),
                prop(
                    "mode",
                    PropType::LiteralUnion { members: vec!["a".into(), "b".into()], has_default: false },
                    Some("A"),
                ),
            ],
            vec![],
        );
        entry.discriminant_prop = Some("mode".into());
        let toon = render_component_toon(&entry);
        assert!(toon.contains("\n  discriminant: mode\n  props[3] "), "{toon}");
        let plain = render_component_toon(&entry_with(vec![], vec![]));
        assert!(!plain.contains("discriminant"), "{plain}");
    }
}
