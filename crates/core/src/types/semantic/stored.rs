//! A [`SemanticOutcome`] as bytes and back, for the checker's answer kept on disk between whole-project runs
//! (`caliper-docgen-semantic`'s answer cache). The bytes are MessagePack of a mirror of the output types, each prop
//! stored once however many components share it (the components hold indices), and they start with this build's
//! [`STORED_FORMAT`] and source fingerprint: bytes written by another build do not decode.
//!
//! The mirror is converted to and from the output types by exhaustive matches, so a variant or field added to a type
//! here fails to compile until it is stored too, and the round trip gives back an equal outcome, props shared as they
//! were (`stored::tests`).

use std::collections::BTreeMap;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use super::{SemanticComponent, SemanticFallback, SemanticOutcome, SemanticProp};
use crate::types::diagnostic::Diagnostic;
use crate::types::output::{
    BigIntValue, CallSignature, EnumMemberValue, FunctionParam, ObjectField, OpaqueDetail, OpaqueReason, PropParent,
    PropType, SignatureKind, TemplatePart, TruncationLimit, TupleElement, TupleElementKind, TupleLabel,
};

/// Bump when the stored shape changes in a way the source fingerprint would not show (it covers this crate's sources,
/// so in practice any change to these types already discards what an older build stored).
const STORED_FORMAT: u32 = 1;

#[derive(Serialize, Deserialize, PartialEq)]
struct Header {
    format: u32,
    fingerprint: String,
}

impl Header {
    fn current() -> Header {
        Header { format: STORED_FORMAT, fingerprint: env!("CALIPER_SOURCE_FINGERPRINT").to_owned() }
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    props: Vec<StoredProp>,
    components: Vec<StoredComponent>,
    fallbacks: Vec<(String, String)>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize, Deserialize)]
struct StoredComponent {
    key: String,
    props: Vec<u32>,
    composes: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
struct StoredProp {
    name: String,
    prop_type: Ty,
    required: bool,
    description: String,
    tags: BTreeMap<String, String>,
    parent: Option<(String, String)>,
    declarations: Vec<(String, String)>,
    react_declared: bool,
}

/// [`PropType`], stored.
#[derive(Serialize, Deserialize)]
enum Ty {
    String,
    Number,
    Boolean,
    Null,
    Undefined,
    Any,
    Never,
    Unknown,
    Void,
    BigInt,
    StringLiteral(String),
    NumberLiteral(f64),
    BoolLiteral(bool),
    BigIntLiteral(String),
    UniqueSymbol(String),
    EnumMember { enum_name: String, member: String, value: EnumValue },
    TemplateLiteral(Vec<Part>),
    Union(Vec<Ty>),
    Intersection(Vec<Ty>),
    Array(Box<Ty>),
    Tuple(Vec<Element>),
    Readonly(Box<Ty>),
    Object(Vec<Field>),
    Callable { signatures: Vec<Signature>, fields: Vec<Field> },
    Named { name: String, args: Vec<Ty> },
    Recursive { name: String, args: Vec<Ty> },
    Truncated { text: String, limit: Limit },
    ReactNode,
    CssProperties,
    Function { type_params: Vec<String>, params: Vec<Param>, return_type: Box<Ty> },
    Ref { name: String, element: Option<String> },
    ElementType,
    HtmlAttributes { element: String, omitted: Vec<String> },
    LiteralUnion { members: Vec<String>, has_default: bool },
    SxProps,
    Opaque { raw: String, reason: Reason },
}

#[derive(Serialize, Deserialize)]
enum EnumValue {
    String(String),
    Number(f64),
}

#[derive(Serialize, Deserialize)]
enum Part {
    Text(String),
    Type(Ty),
}

#[derive(Serialize, Deserialize)]
struct Element {
    element_type: Ty,
    kind: u8,
    label: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Field {
    name: String,
    prop_type: Ty,
    required: bool,
    description: String,
    readonly: bool,
    index_key: Option<Box<Ty>>,
}

#[derive(Serialize, Deserialize)]
struct Signature {
    construct: bool,
    type_params: Vec<String>,
    params: Vec<Param>,
    return_type: Ty,
}

#[derive(Serialize, Deserialize)]
struct Param {
    name: Option<String>,
    param_type: Ty,
    optional: bool,
    rest: bool,
}

#[derive(Serialize, Deserialize)]
enum Limit {
    Depth,
    Members(u32),
}

#[derive(Serialize, Deserialize)]
enum Reason {
    ConditionalType,
    MappedType,
    ModuleAugmentation,
    RuntimeDependent(String),
    UnresolvableImport(String),
    PandaCodegenMissing,
    DepthExceeded,
    IndexedAccess(String),
    UnsupportedExpression,
}

impl SemanticOutcome {
    /// The outcome as bytes [`SemanticOutcome::from_bytes`] reads back in a process of the same build.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut index: FxHashMap<*const SemanticProp, u32> = FxHashMap::default();
        let mut props = Vec::new();
        let mut components = Vec::with_capacity(self.components.len());
        for component in &self.components {
            let mut held = Vec::with_capacity(component.props.len());
            for prop in &component.props {
                let next = u32::try_from(props.len()).map_err(|_| "too many props to store".to_string())?;
                let at = *index.entry(Arc::as_ptr(prop)).or_insert_with(|| {
                    props.push(store_prop(prop));
                    next
                });
                held.push(at);
            }
            components.push(StoredComponent {
                key: component.key.clone(),
                props: held,
                composes: component.composes.clone(),
            });
        }
        let stored = Stored {
            props,
            components,
            fallbacks: self.fallbacks.iter().map(|f| (f.key.clone(), f.reason.clone())).collect(),
            diagnostics: self.diagnostics.clone(),
        };
        let mut bytes = rmp_serde::to_vec(&Header::current()).map_err(|error| error.to_string())?;
        bytes.extend(rmp_serde::to_vec(&stored).map_err(|error| error.to_string())?);
        Ok(bytes)
    }

    /// The outcome [`SemanticOutcome::to_bytes`] wrote, each stored prop shared by the components that held it; an
    /// error for bytes another build wrote, or bytes that are not an outcome.
    pub fn from_bytes(bytes: &[u8]) -> Result<SemanticOutcome, String> {
        let mut decoder = rmp_serde::Deserializer::new(bytes);
        match Header::deserialize(&mut decoder) {
            Ok(header) if header == Header::current() => {}
            Ok(_) => return Err("written by another build".to_string()),
            Err(error) => return Err(error.to_string()),
        }
        let stored = Stored::deserialize(&mut decoder).map_err(|error| error.to_string())?;
        let props: Vec<Arc<SemanticProp>> =
            stored.props.into_iter().map(|prop| load_prop(prop).map(Arc::new)).collect::<Result<_, _>>()?;
        let components = stored
            .components
            .into_iter()
            .map(|component| {
                let held = component
                    .props
                    .iter()
                    .map(|at| {
                        let at = usize::try_from(*at).map_err(|_| "a prop index out of range".to_string())?;
                        props.get(at).cloned().ok_or_else(|| "a prop index out of range".to_string())
                    })
                    .collect::<Result<_, String>>()?;
                Ok(SemanticComponent { key: component.key, props: held, composes: component.composes })
            })
            .collect::<Result<_, String>>()?;
        Ok(SemanticOutcome {
            components,
            fallbacks: stored.fallbacks.into_iter().map(|(key, reason)| SemanticFallback { key, reason }).collect(),
            diagnostics: stored.diagnostics,
        })
    }
}

fn store_prop(prop: &SemanticProp) -> StoredProp {
    let parent = |parent: &PropParent| (parent.name.clone(), parent.file_name.clone());
    StoredProp {
        name: prop.name.clone(),
        prop_type: store(&prop.prop_type),
        required: prop.required,
        description: prop.description.clone(),
        tags: prop.tags.clone(),
        parent: prop.parent.as_ref().map(parent),
        declarations: prop.declarations.iter().map(parent).collect(),
        react_declared: prop.react_declared,
    }
}

fn load_prop(prop: StoredProp) -> Result<SemanticProp, String> {
    let parent = |(name, file_name): (String, String)| PropParent { name, file_name };
    Ok(SemanticProp {
        name: prop.name,
        prop_type: load(prop.prop_type)?,
        required: prop.required,
        description: prop.description,
        tags: prop.tags,
        parent: prop.parent.map(parent),
        declarations: prop.declarations.into_iter().map(parent).collect(),
        react_declared: prop.react_declared,
    })
}

fn store_all(types: &[PropType]) -> Vec<Ty> {
    types.iter().map(store).collect()
}

fn store_fields(fields: &[ObjectField]) -> Vec<Field> {
    fields
        .iter()
        .map(|field| Field {
            name: field.name.clone(),
            prop_type: store(&field.prop_type),
            required: field.required,
            description: field.description.clone(),
            readonly: field.readonly,
            index_key: field.index_key.as_deref().map(|key| Box::new(store(key))),
        })
        .collect()
}

fn store_params(params: &[FunctionParam]) -> Vec<Param> {
    params
        .iter()
        .map(|param| Param {
            name: param.name.clone(),
            param_type: store(&param.param_type),
            optional: param.optional,
            rest: param.rest,
        })
        .collect()
}

fn store(ty: &PropType) -> Ty {
    match ty {
        PropType::String => Ty::String,
        PropType::Number => Ty::Number,
        PropType::Boolean => Ty::Boolean,
        PropType::Null => Ty::Null,
        PropType::Undefined => Ty::Undefined,
        PropType::Any => Ty::Any,
        PropType::Never => Ty::Never,
        PropType::Unknown => Ty::Unknown,
        PropType::Void => Ty::Void,
        PropType::BigInt => Ty::BigInt,
        PropType::StringLiteral(value) => Ty::StringLiteral(value.clone()),
        PropType::NumberLiteral(value) => Ty::NumberLiteral(*value),
        PropType::BoolLiteral(value) => Ty::BoolLiteral(*value),
        PropType::BigIntLiteral(value) => Ty::BigIntLiteral(value.as_str().to_string()),
        PropType::UniqueSymbol { name } => Ty::UniqueSymbol(name.to_string()),
        PropType::EnumMember { enum_name, member, value } => Ty::EnumMember {
            enum_name: enum_name.to_string(),
            member: member.to_string(),
            value: match value {
                EnumMemberValue::String(text) => EnumValue::String(text.clone()),
                EnumMemberValue::Number(number) => EnumValue::Number(*number),
            },
        },
        PropType::TemplateLiteral(parts) => Ty::TemplateLiteral(
            parts
                .iter()
                .map(|part| match part {
                    TemplatePart::Text(text) => Part::Text(text.clone()),
                    TemplatePart::Type(ty) => Part::Type(store(ty)),
                })
                .collect(),
        ),
        PropType::Union(members) => Ty::Union(store_all(members)),
        PropType::Intersection(members) => Ty::Intersection(store_all(members)),
        PropType::Array(inner) => Ty::Array(Box::new(store(inner))),
        PropType::Tuple(elements) => Ty::Tuple(
            elements
                .iter()
                .map(|element| Element {
                    element_type: store(&element.element_type),
                    kind: match element.kind {
                        TupleElementKind::Required => 0,
                        TupleElementKind::Optional => 1,
                        TupleElementKind::Rest => 2,
                        TupleElementKind::Variadic => 3,
                    },
                    label: element.label.as_ref().map(|label| label.as_str().to_string()),
                })
                .collect(),
        ),
        PropType::Readonly(inner) => Ty::Readonly(Box::new(store(inner))),
        PropType::Object(fields) => Ty::Object(store_fields(fields)),
        PropType::Callable { signatures, fields } => Ty::Callable {
            signatures: signatures
                .iter()
                .map(|signature| Signature {
                    construct: signature.kind == SignatureKind::Construct,
                    type_params: signature.type_params.clone(),
                    params: store_params(&signature.params),
                    return_type: store(&signature.return_type),
                })
                .collect(),
            fields: store_fields(fields),
        },
        PropType::Named { name, args } => Ty::Named { name: name.to_string(), args: store_all(args) },
        PropType::Recursive { name, args } => Ty::Recursive { name: name.to_string(), args: store_all(args) },
        PropType::Truncated { text, limit } => Ty::Truncated {
            text: text.clone(),
            limit: match limit {
                TruncationLimit::Depth => Limit::Depth,
                TruncationLimit::Members { total } => Limit::Members(*total),
            },
        },
        PropType::ReactNode => Ty::ReactNode,
        PropType::CssProperties => Ty::CssProperties,
        PropType::Function { type_params, params, return_type } => Ty::Function {
            type_params: type_params.clone(),
            params: store_params(params),
            return_type: Box::new(store(return_type)),
        },
        PropType::Ref { name, element } => Ty::Ref { name: name.clone(), element: element.clone() },
        PropType::ElementType => Ty::ElementType,
        PropType::HtmlAttributes { element, omitted } => {
            Ty::HtmlAttributes { element: element.clone(), omitted: omitted.clone() }
        }
        PropType::LiteralUnion { members, has_default } => {
            Ty::LiteralUnion { members: members.clone(), has_default: *has_default }
        }
        PropType::SxProps => Ty::SxProps,
        PropType::Opaque(detail) => Ty::Opaque {
            raw: detail.raw().to_string(),
            reason: match detail.reason() {
                OpaqueReason::ConditionalType => Reason::ConditionalType,
                OpaqueReason::MappedType => Reason::MappedType,
                OpaqueReason::ModuleAugmentation => Reason::ModuleAugmentation,
                OpaqueReason::RuntimeDependent { function_name } => Reason::RuntimeDependent(function_name.clone()),
                OpaqueReason::UnresolvableImport { specifier } => Reason::UnresolvableImport(specifier.clone()),
                OpaqueReason::PandaCodegenMissing => Reason::PandaCodegenMissing,
                OpaqueReason::DepthExceeded => Reason::DepthExceeded,
                OpaqueReason::IndexedAccess { expression } => Reason::IndexedAccess(expression.clone()),
                OpaqueReason::UnsupportedExpression => Reason::UnsupportedExpression,
            },
        },
    }
}

fn load_all(types: Vec<Ty>) -> Result<Vec<PropType>, String> {
    types.into_iter().map(load).collect()
}

fn load_fields(fields: Vec<Field>) -> Result<Vec<ObjectField>, String> {
    fields
        .into_iter()
        .map(|field| {
            Ok(ObjectField {
                name: field.name,
                prop_type: load(field.prop_type)?,
                required: field.required,
                description: field.description,
                readonly: field.readonly,
                index_key: field.index_key.map(|key| load(*key).map(Box::new)).transpose()?,
            })
        })
        .collect()
}

fn load_params(params: Vec<Param>) -> Result<Vec<FunctionParam>, String> {
    params
        .into_iter()
        .map(|param| {
            Ok(FunctionParam {
                name: param.name,
                param_type: load(param.param_type)?,
                optional: param.optional,
                rest: param.rest,
            })
        })
        .collect()
}

fn load(ty: Ty) -> Result<PropType, String> {
    Ok(match ty {
        Ty::String => PropType::String,
        Ty::Number => PropType::Number,
        Ty::Boolean => PropType::Boolean,
        Ty::Null => PropType::Null,
        Ty::Undefined => PropType::Undefined,
        Ty::Any => PropType::Any,
        Ty::Never => PropType::Never,
        Ty::Unknown => PropType::Unknown,
        Ty::Void => PropType::Void,
        Ty::BigInt => PropType::BigInt,
        Ty::StringLiteral(value) => PropType::StringLiteral(value),
        Ty::NumberLiteral(value) => PropType::NumberLiteral(value),
        Ty::BoolLiteral(value) => PropType::BoolLiteral(value),
        Ty::BigIntLiteral(value) => PropType::BigIntLiteral(
            BigIntValue::parse(&value).ok_or_else(|| format!("a stored bigint `{value}` that is not one"))?,
        ),
        Ty::UniqueSymbol(name) => PropType::UniqueSymbol { name: name.into() },
        Ty::EnumMember { enum_name, member, value } => PropType::EnumMember {
            enum_name: enum_name.into(),
            member: member.into(),
            value: match value {
                EnumValue::String(text) => EnumMemberValue::String(text),
                EnumValue::Number(number) => EnumMemberValue::Number(number),
            },
        },
        Ty::TemplateLiteral(parts) => PropType::TemplateLiteral(
            parts
                .into_iter()
                .map(|part| {
                    Ok(match part {
                        Part::Text(text) => TemplatePart::Text(text),
                        Part::Type(ty) => TemplatePart::Type(load(ty)?),
                    })
                })
                .collect::<Result<_, String>>()?,
        ),
        Ty::Union(members) => PropType::Union(load_all(members)?),
        Ty::Intersection(members) => PropType::Intersection(load_all(members)?),
        Ty::Array(inner) => PropType::Array(Box::new(load(*inner)?)),
        Ty::Tuple(elements) => PropType::Tuple(
            elements
                .into_iter()
                .map(|element| {
                    let label = match element.label {
                        Some(label) => {
                            Some(TupleLabel::new(&label).ok_or_else(|| format!("a stored tuple label `{label}`"))?)
                        }
                        None => None,
                    };
                    let kind = match element.kind {
                        0 => TupleElementKind::Required,
                        1 => TupleElementKind::Optional,
                        2 => TupleElementKind::Rest,
                        3 => TupleElementKind::Variadic,
                        other => return Err(format!("a stored tuple element kind {other}")),
                    };
                    Ok(TupleElement { element_type: load(element.element_type)?, kind, label })
                })
                .collect::<Result<_, String>>()?,
        ),
        Ty::Readonly(inner) => PropType::Readonly(Box::new(load(*inner)?)),
        Ty::Object(fields) => PropType::Object(load_fields(fields)?),
        Ty::Callable { signatures, fields } => PropType::Callable {
            signatures: signatures
                .into_iter()
                .map(|signature| {
                    Ok(CallSignature {
                        kind: if signature.construct { SignatureKind::Construct } else { SignatureKind::Call },
                        type_params: signature.type_params,
                        params: load_params(signature.params)?,
                        return_type: load(signature.return_type)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            fields: load_fields(fields)?,
        },
        Ty::Named { name, args } => PropType::Named { name: name.into(), args: load_all(args)? },
        Ty::Recursive { name, args } => PropType::Recursive { name: name.into(), args: load_all(args)? },
        Ty::Truncated { text, limit } => PropType::Truncated {
            text,
            limit: match limit {
                Limit::Depth => TruncationLimit::Depth,
                Limit::Members(total) => TruncationLimit::Members { total },
            },
        },
        Ty::ReactNode => PropType::ReactNode,
        Ty::CssProperties => PropType::CssProperties,
        Ty::Function { type_params, params, return_type } => {
            PropType::Function { type_params, params: load_params(params)?, return_type: Box::new(load(*return_type)?) }
        }
        Ty::Ref { name, element } => PropType::Ref { name, element },
        Ty::ElementType => PropType::ElementType,
        Ty::HtmlAttributes { element, omitted } => PropType::HtmlAttributes { element, omitted },
        Ty::LiteralUnion { members, has_default } => PropType::LiteralUnion { members, has_default },
        Ty::SxProps => PropType::SxProps,
        Ty::Opaque { raw, reason } => {
            let reason = match reason {
                Reason::ConditionalType => OpaqueReason::ConditionalType,
                Reason::MappedType => OpaqueReason::MappedType,
                Reason::ModuleAugmentation => OpaqueReason::ModuleAugmentation,
                Reason::RuntimeDependent(function_name) => OpaqueReason::RuntimeDependent { function_name },
                Reason::UnresolvableImport(specifier) => OpaqueReason::UnresolvableImport { specifier },
                Reason::PandaCodegenMissing => OpaqueReason::PandaCodegenMissing,
                Reason::DepthExceeded => OpaqueReason::DepthExceeded,
                Reason::IndexedAccess(expression) => OpaqueReason::IndexedAccess { expression },
                Reason::UnsupportedExpression => OpaqueReason::UnsupportedExpression,
            };
            #[allow(clippy::disallowed_methods, reason = "a stored answer's degradation was reported when it was made")]
            OpaqueDetail::new(raw, reason)
        }
    })
}

#[cfg(test)]
mod tests;
