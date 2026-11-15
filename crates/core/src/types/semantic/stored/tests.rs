#![allow(clippy::unwrap_used, clippy::disallowed_methods)]

use super::*;
use crate::types::diagnostic::{DiagnosticCode, DiagnosticSeverity};

/// One of every kind of type, nested where a kind holds others.
fn every_type() -> PropType {
    let field = |name: &str, ty: PropType| ObjectField::new(name, ty, true, "a field");
    let mut indexed = field("key", PropType::Number);
    indexed.readonly = true;
    indexed.index_key = Some(Box::new(PropType::String));
    let mut rest = FunctionParam::new(Some("rest".into()), PropType::Array(Box::new(PropType::String)));
    rest.rest = true;
    let mut optional = FunctionParam::new(None, PropType::Boolean);
    optional.optional = true;
    let reasons = [
        OpaqueReason::ConditionalType,
        OpaqueReason::MappedType,
        OpaqueReason::ModuleAugmentation,
        OpaqueReason::RuntimeDependent { function_name: "f".into() },
        OpaqueReason::UnresolvableImport { specifier: "./x".into() },
        OpaqueReason::PandaCodegenMissing,
        OpaqueReason::DepthExceeded,
        OpaqueReason::IndexedAccess { expression: "T[K]".into() },
        OpaqueReason::UnsupportedExpression,
    ];
    let mut members = vec![
        PropType::String,
        PropType::Number,
        PropType::Boolean,
        PropType::Null,
        PropType::Undefined,
        PropType::Any,
        PropType::Never,
        PropType::Unknown,
        PropType::Void,
        PropType::BigInt,
        PropType::StringLiteral("sm".into()),
        PropType::NumberLiteral(0.1),
        PropType::NumberLiteral(f64::NAN),
        PropType::NumberLiteral(f64::NEG_INFINITY),
        PropType::BoolLiteral(true),
        PropType::BigIntLiteral(BigIntValue::parse("-10").unwrap()),
        PropType::UniqueSymbol { name: "brand".into() },
        PropType::EnumMember { enum_name: "Size".into(), member: "Sm".into(), value: EnumMemberValue::Number(1.5) },
        PropType::EnumMember {
            enum_name: "Size".into(),
            member: "Lg".into(),
            value: EnumMemberValue::String("lg".into()),
        },
        PropType::TemplateLiteral(vec![TemplatePart::Text("px-".into()), TemplatePart::Type(PropType::Number)]),
        PropType::Intersection(vec![PropType::Named { name: "A".into(), args: vec![PropType::String] }]),
        PropType::Tuple(vec![
            TupleElement::required(PropType::String),
            TupleElement {
                element_type: PropType::Number,
                kind: TupleElementKind::Optional,
                label: TupleLabel::new("count"),
            },
            TupleElement { element_type: PropType::Number, kind: TupleElementKind::Rest, label: None },
            TupleElement { element_type: PropType::Unknown, kind: TupleElementKind::Variadic, label: None },
        ]),
        PropType::Readonly(Box::new(PropType::Array(Box::new(PropType::String)))),
        PropType::Object(vec![field("a", PropType::String), indexed.clone()]),
        PropType::Callable {
            signatures: vec![
                CallSignature {
                    kind: SignatureKind::Call,
                    type_params: vec!["T".into()],
                    params: vec![rest.clone(), optional.clone()],
                    return_type: PropType::Void,
                },
                CallSignature {
                    kind: SignatureKind::Construct,
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: PropType::Named { name: "Date".into(), args: Vec::new() },
                },
            ],
            fields: vec![indexed],
        },
        PropType::Recursive { name: "Json".into(), args: vec![PropType::Any] },
        PropType::Truncated { text: "Huge".into(), limit: TruncationLimit::Depth },
        PropType::Truncated { text: "Wide".into(), limit: TruncationLimit::Members { total: 900 } },
        PropType::ReactNode,
        PropType::CssProperties,
        PropType::Function {
            type_params: vec!["T".into()],
            params: vec![rest, optional],
            return_type: Box::new(PropType::ReactNode),
        },
        PropType::Ref { name: "RefObject".into(), element: Some("HTMLDivElement".into()) },
        PropType::Ref { name: "Ref".into(), element: None },
        PropType::ElementType,
        PropType::HtmlAttributes { element: "button".into(), omitted: vec!["color".into()] },
        PropType::LiteralUnion { members: vec!["a".into(), "b".into()], has_default: true },
        PropType::SxProps,
    ];
    members.extend(reasons.into_iter().map(|reason| OpaqueDetail::new("raw", reason)));
    PropType::Union(members)
}

fn prop(name: &str, prop_type: PropType) -> SemanticProp {
    let parent = PropParent { name: "Props".into(), file_name: "/p/props.ts".into() };
    SemanticProp {
        name: name.into(),
        prop_type,
        required: false,
        description: "What it is.".into(),
        tags: BTreeMap::from([("since".to_string(), "2.0".to_string())]),
        parent: Some(parent.clone()),
        declarations: vec![parent],
        react_declared: true,
    }
}

#[test]
fn an_outcome_reads_back_equal_with_its_props_shared_as_they_were() {
    let shared = Arc::new(prop("everything", every_type()));
    let own = Arc::new(prop("own", PropType::String));
    let outcome = SemanticOutcome {
        components: vec![
            SemanticComponent {
                key: "A".into(),
                props: vec![shared.clone(), own],
                composes: Some(vec!["RefAttributes".into()]),
            },
            SemanticComponent { key: "B".into(), props: vec![shared], composes: None },
        ],
        fallbacks: vec![SemanticFallback { key: "C".into(), reason: "overloaded call signatures".into() }],
        diagnostics: vec![Diagnostic::new(DiagnosticSeverity::Info, DiagnosticCode::SemanticBackend, "kept")],
    };

    let read = SemanticOutcome::from_bytes(&outcome.to_bytes().unwrap()).unwrap();

    // NaN is not equal to itself, so the types are compared as the output writes them.
    let written = |outcome: &SemanticOutcome| {
        serde_json::to_string(
            &outcome.components.iter().flat_map(|c| c.props.iter().map(|p| &p.prop_type)).collect::<Vec<_>>(),
        )
        .unwrap()
    };
    assert_eq!(written(&read), written(&outcome));
    assert_eq!(read.components.len(), 2);
    assert_eq!(
        (read.fallbacks.clone(), read.diagnostics.clone()),
        (outcome.fallbacks.clone(), outcome.diagnostics.clone())
    );
    for (read, original) in read.components.iter().zip(&outcome.components) {
        assert_eq!((&read.key, &read.composes), (&original.key, &original.composes));
        for (read, original) in read.props.iter().zip(&original.props) {
            assert_eq!(
                (
                    &read.name,
                    read.required,
                    &read.description,
                    &read.tags,
                    &read.parent,
                    &read.declarations,
                    read.react_declared
                ),
                (
                    &original.name,
                    original.required,
                    &original.description,
                    &original.tags,
                    &original.parent,
                    &original.declarations,
                    original.react_declared
                )
            );
        }
    }
    assert!(Arc::ptr_eq(&read.components[0].props[0], &read.components[1].props[0]));
}

#[test]
fn bytes_another_build_wrote_do_not_read() {
    let mut header = rmp_serde::to_vec(&Header { format: STORED_FORMAT, fingerprint: "another".into() }).unwrap();
    header.extend(
        rmp_serde::to_vec(&Stored {
            props: Vec::new(),
            components: Vec::new(),
            fallbacks: Vec::new(),
            diagnostics: Vec::new(),
        })
        .unwrap(),
    );

    assert_eq!(SemanticOutcome::from_bytes(&header).err().as_deref(), Some("written by another build"));
    assert!(SemanticOutcome::from_bytes(b"not an outcome").is_err());
}
