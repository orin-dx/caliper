//! HTML attribute inference helpers.

use std::collections::VecDeque;

use camino::Utf8Path;

use crate::types::*;

use super::collected::resolve_collected_type;
use super::substitute::{build_substitution, substitute_alias, substitute_type};
use super::ResolutionContext;

/// `JSX.IntrinsicElements[element]` in React's own types at `react_file`, when this run has them: Full mode merges them;
/// curated mode and the dependency loader leave them to the HTML layer.
pub(super) fn intrinsic_element<'a>(
    element: &str,
    react_file: &Utf8Path,
    ctx: &'a ResolutionContext,
) -> Option<&'a CollectedType> {
    let elements = ctx.named_types.lookup_interface_exact(react_file, "React.JSX.IntrinsicElements")?;
    Some(&elements.props.iter().find(|prop| prop.name == element)?.collected_type)
}

/// The declaration of attribute `name` on intrinsic element `element`, read as TypeScript reads
/// `JSX.IntrinsicElements[element]` (`intrinsic_element`). `None` without React's types, or when the element has no such
/// attribute.
pub(super) fn react_attribute(
    element: &str,
    name: &str,
    react_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Option<ParsedProp> {
    // Declared inside `declare namespace React`, so keyed by their qualified names whichever way they are referenced.
    let key = |type_name: &str| format!("React.{}", type_name.strip_prefix("React.").unwrap_or(type_name));
    let mut pending = VecDeque::from([intrinsic_element(element, react_file, ctx)?.clone()]);
    // React's attribute types are a small tree; the bound only guards against a malformed declaration file.
    for _ in 0..64 {
        match pending.pop_front()? {
            CollectedType::AtFile { inner, .. } => pending.push_back(*inner),
            CollectedType::Intersection(members) => pending.extend(members),
            CollectedType::Named { name: type_name, args } => {
                let key = key(&type_name);
                // Defaulted parameters (`T = Element`) are not supplied, and need no warning.
                let mut ignored = Vec::new();
                if let Some(iface) = ctx.named_types.lookup_interface_exact(react_file, &key) {
                    let params = ctx.named_types.lookup_interface_type_params_for(iface);
                    let subst = params.map(|params| build_substitution(params, &args, react_file, &mut ignored));
                    let substituted =
                        |ty: &CollectedType| subst.as_ref().map_or_else(|| ty.clone(), |s| substitute_type(ty, s));
                    if let Some(raw) = iface.props.iter().find(|prop| prop.name == name) {
                        let ty = substituted(&raw.collected_type);
                        let prop_type = resolve_collected_type(&ty, &iface.file_path, ctx, state, depth);
                        let parent =
                            PropParent { name: iface.name.to_string(), file_name: iface.file_path.to_string() };
                        return Some(ParsedProp::new(
                            name.to_owned(),
                            prop_type,
                            raw.required,
                            None,
                            raw.description.clone(),
                            raw.tags.clone(),
                            Some(parent.clone()),
                            vec![parent],
                        ));
                    }
                    pending.extend(iface.extends.iter().map(|extends| {
                        let (ExtendsRef::SameFile { name, type_args }
                        | ExtendsRef::Imported { local_name: name, type_args, .. }
                        | ExtendsRef::Builtin { name, type_args, .. }) = extends;
                        CollectedType::Named { name: name.clone(), args: type_args.iter().map(substituted).collect() }
                    }));
                } else if let Some(alias) = ctx.named_types.lookup_type_alias_exact(react_file, &key) {
                    let alias = match ctx.named_types.lookup_type_alias_params(react_file, &key) {
                        Some(params) => {
                            substitute_alias(alias, &build_substitution(params, &args, react_file, &mut ignored))
                        }
                        None => alias.clone(),
                    };
                    match alias {
                        CollectedTypeAlias::Intersection { members, .. } => pending.extend(members),
                        CollectedTypeAlias::Passthrough { target, .. } => pending.push_back(target),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn infer_html_attr_prop_type(attr_name: &str) -> PropType {
    match attr_name {
        "onClick" | "onKeyDown" | "onKeyUp" | "onFocus" | "onBlur" | "onChange" | "onInput" | "onSubmit"
        | "onReset" | "onLoad" | "onError" | "onPress" | "onPressStart" | "onPressEnd" | "onHoverStart"
        | "onHoverEnd" | "onFocusChange" | "onPressChange" => PropType::Function {
            type_params: vec![],
            params: vec![FunctionParam {
                name: Some("e".into()),
                param_type: PropType::Named { name: "Event".into(), args: vec![] },
            }],
            return_type: Box::new(PropType::Void),
        },
        "disabled" | "readOnly" | "required" | "checked" | "multiple" | "noValidate" | "autoFocus" | "fullWidth"
        | "loading" | "isDisabled" | "isReadOnly" | "isRequired" => PropType::Boolean,
        "tabIndex" | "rows" | "cols" | "maxLength" | "min" | "max" | "width" | "height" | "size" => PropType::Number,
        "style" => PropType::CssProperties,
        "children" => PropType::ReactNode,
        _ => PropType::String,
    }
}

pub(super) fn capitalize_element(element: &str) -> &'static str {
    match element {
        "button" => "Button",
        "input" => "Input",
        "a" => "Anchor",
        "textarea" => "Textarea",
        "select" => "Select",
        "form" => "Form",
        "label" => "Label",
        "img" => "Img",
        "video" => "Video",
        "audio" => "Audio",
        _ => "HTML",
    }
}
