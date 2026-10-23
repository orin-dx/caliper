//! Function type and typeof resolution.

use camino::Utf8Path;
use compact_str::CompactString;

use crate::types::*;

use super::ResolutionContext;

/// A function type with its parameters and return type as written. A signature names its types (`(mode: CalendarMode)
/// => void`) the way TypeScript prints it; expanding each alias into its definition would read worse and, for a
/// conditional or mapped alias, would leave raw type text in place of the name.
pub(super) fn resolve_function_type(
    type_params: &[CompactString],
    params: &[CollectedType],
    param_names: &[Option<CompactString>],
    return_type: &CollectedType,
) -> PropType {
    let params = params
        .iter()
        .enumerate()
        .map(|(i, param)| FunctionParam {
            name: param_names.get(i).cloned().flatten().map(|n| n.to_string()),
            param_type: as_written(param),
        })
        .collect();
    PropType::Function {
        type_params: type_params.iter().map(|p| p.to_string()).collect(),
        params,
        return_type: Box::new(as_written(return_type)),
    }
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
        CollectedType::Union(members) => PropType::Union(members.iter().map(as_written).collect()),
        CollectedType::Intersection(members) => PropType::Intersection(members.iter().map(as_written).collect()),
        CollectedType::Array(inner) => PropType::Array(Box::new(as_written(inner))),
        CollectedType::Tuple(members) => PropType::Tuple(members.iter().map(as_written).collect()),
        CollectedType::Object(fields) => PropType::Object(
            fields
                .iter()
                .map(|f| ObjectField {
                    name: f.name.clone(),
                    prop_type: as_written(&f.collected_type),
                    required: f.required,
                    description: f.description.clone(),
                })
                .collect(),
        ),
        CollectedType::Named { name, args } => {
            PropType::Named { name: name.clone(), args: args.iter().map(as_written).collect() }
        }
        CollectedType::Function { type_params, params, param_names, return_type } => {
            resolve_function_type(type_params, params, param_names, return_type)
        }
        CollectedType::AtFile { inner, .. } => as_written(inner),
        CollectedType::BigInt
        | CollectedType::Symbol
        | CollectedType::TypeOf(_)
        | CollectedType::KeyOf(_)
        | CollectedType::IndexedAccess { .. }
        | CollectedType::TemplateLiteral(_)
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
    let found_enum = ctx.enum_bare_index.get(name.as_str()).and_then(|key| ctx.global.enums.get(key.as_str()));

    if found_enum.is_some() {
        // Has cva-like enum entries — the VariantProps<typeof X> pattern handles this.
        // At the type level, surface as Named.
        return PropType::Named { name: name.clone(), args: vec![] };
    }

    diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!("typeof '{}' in '{}' — could not statically evaluate", name, consuming_file),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::OpaqueType,
        affects: Vec::new(),
    });

    // The value's type, not the value: spelled as written, as TypeScript prints it.
    PropType::Named { name: format!("typeof {name}").into(), args: vec![] }
}
