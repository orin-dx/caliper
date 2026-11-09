//! Function type and typeof resolution.

use camino::Utf8Path;
use compact_str::CompactString;

use crate::types::*;

use super::ResolutionContext;
use super::primitives::union_of;

/// A function type with its parameters and return type as written. A signature names its types (`(mode: CalendarMode)
/// => void`) the way TypeScript prints it; expanding each alias into its definition would read worse and, for a
/// conditional or mapped alias, would leave raw type text in place of the name.
pub(super) fn resolve_function_type(
    type_params: &[CompactString],
    params: &[CollectedType],
    param_names: &[Option<CompactString>],
    param_kinds: &[ParamKind],
    return_type: &CollectedType,
) -> PropType {
    PropType::Function {
        type_params: type_params.iter().map(|p| p.to_string()).collect(),
        params: params_as_written(params, param_names, param_kinds),
        return_type: Box::new(as_written(return_type)),
    }
}

/// A call or construct signature of an object type, as written like a function type's.
pub(super) fn resolve_signature(signature: &CollectedSignature) -> CallSignature {
    CallSignature {
        kind: signature.kind,
        type_params: signature.type_params.iter().map(|p| p.to_string()).collect(),
        params: params_as_written(&signature.params, &signature.param_names, &signature.param_kinds),
        return_type: as_written(&signature.return_type),
    }
}

/// A signature's parameters, each type as written.
fn params_as_written(
    params: &[CollectedType],
    param_names: &[Option<CompactString>],
    param_kinds: &[ParamKind],
) -> Vec<FunctionParam> {
    params
        .iter()
        .enumerate()
        .map(|(i, param)| {
            let kind = param_kinds.get(i).copied().unwrap_or_default();
            FunctionParam {
                name: param_names.get(i).cloned().flatten().map(|n| n.to_string()),
                param_type: as_written(param),
                optional: kind == ParamKind::Optional,
                rest: kind == ParamKind::Rest,
            }
        })
        .collect()
}

/// An object type's fields, each type as written.
fn fields_as_written(fields: &[CollectedObjectField]) -> Vec<ObjectField> {
    fields
        .iter()
        .map(|f| ObjectField {
            name: f.name.clone(),
            prop_type: as_written(&f.collected_type),
            required: f.required,
            description: f.description.clone(),
            readonly: f.readonly,
            index_key: f.index_key.as_deref().map(|key| Box::new(as_written(key))),
        })
        .collect()
}

/// `ct` as its source spells it: names stay names, and a form `PropType` has no shape for keeps its source text.
pub(super) fn as_written(ct: &CollectedType) -> PropType {
    let named = |name: std::string::String| PropType::Named { name: name.into(), args: vec![] };
    match ct {
        CollectedType::String => PropType::String,
        CollectedType::Number => PropType::Number,
        CollectedType::Boolean => PropType::Boolean,
        CollectedType::Null => PropType::Null,
        CollectedType::Undefined => PropType::Undefined,
        CollectedType::Any => PropType::Any,
        CollectedType::Never => PropType::Never,
        CollectedType::Unknown => PropType::Unknown,
        CollectedType::Void => PropType::Void,
        CollectedType::StringLiteral(s) => PropType::StringLiteral(s.to_string()),
        CollectedType::NumberLiteral(n) => PropType::NumberLiteral(*n),
        CollectedType::BoolLiteral(b) => PropType::BoolLiteral(*b),
        CollectedType::BigIntLiteral(value) => super::collected::big_int_literal(value),
        CollectedType::Union(members) => union_of(members.iter().map(as_written)),
        CollectedType::Intersection(members) => PropType::Intersection(members.iter().map(as_written).collect()),
        CollectedType::Array(inner) => PropType::Array(Box::new(as_written(inner))),
        CollectedType::Tuple(elements) => super::collected::tuple_of(elements, as_written),
        CollectedType::Readonly(inner) => PropType::Readonly(Box::new(as_written(inner))),
        CollectedType::Object(fields) => PropType::Object(fields_as_written(fields)),
        CollectedType::Callable { signatures, fields } => PropType::Callable {
            signatures: signatures.iter().map(resolve_signature).collect(),
            fields: fields_as_written(fields),
        },
        CollectedType::Named { name, args } => {
            PropType::Named { name: unmarked(name).into(), args: args.iter().map(as_written).collect() }
        }
        CollectedType::Function { type_params, params, param_names, param_kinds, return_type } => {
            resolve_function_type(type_params, params, param_names, param_kinds, return_type)
        }
        CollectedType::AtFile { inner, .. } => as_written(inner),
        CollectedType::TemplateLiteral(parts) => super::template::template_of(parts, as_written),
        CollectedType::BigInt
        | CollectedType::Symbol
        | CollectedType::UniqueSymbol
        | CollectedType::TypeOf(_)
        | CollectedType::KeyOf(_)
        | CollectedType::IndexedAccess { .. }
        | CollectedType::Conditional { .. }
        | CollectedType::Mapped { .. }
        | CollectedType::Raw(_) => named(ct.to_raw_string()),
    }
}

pub(super) fn resolve_typeof(
    name: &CompactString,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> PropType {
    // `typeof X` — look for X in global.enums (for cva() results), via the
    // precomputed bare-name index (O(1) instead of a linear scan over every
    // enum/cva/tv/recipe entry in the project with a per-candidate allocation).
    super::deps::record_bare_name(name);
    let found_enum = ctx.enum_key_for(name.as_str(), consuming_file).and_then(|key| ctx.global.enums.get(key.as_str()));

    if found_enum.is_some() {
        // Has cva-like enum entries — the VariantProps<typeof X> pattern handles this.
        // At the type level, surface as Named.
        return PropType::Named { name: name.clone(), args: vec![] };
    }

    // `const SHOW_PARENT = 'SHOW_PARENT'`: the literal TypeScript gives a `const`.
    match super::evaluate::value_type(name, consuming_file, ctx) {
        Some(CollectedType::StringLiteral(value)) => return PropType::StringLiteral(value.to_string()),
        Some(CollectedType::NumberLiteral(value)) => return PropType::NumberLiteral(value),
        Some(CollectedType::BoolLiteral(value)) => return PropType::BoolLiteral(value),
        // `declare const brand: unique symbol`: the symbol type only that value has.
        Some(CollectedType::UniqueSymbol) => return PropType::UniqueSymbol { name: name.clone() },
        _ => {}
    }

    diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!("typeof '{}' in '{}' — could not statically evaluate", name, consuming_file),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some(
            "Only components, cva variants and `as const` arrays have a type here. The type of a value built by a \
             call (a zod schema behind `z.infer`, a store, a hook's result) comes from the checker: declare the type \
             where it is used, or run with `--experimental-semantic-types`."
                .into(),
        ),
        code: DiagnosticCode::OpaqueType,
        affects: Vec::new(),
    });

    // The value's type, not the value: spelled as written, as TypeScript prints it.
    PropType::Named { name: format!("typeof {name}").into(), args: vec![] }
}
