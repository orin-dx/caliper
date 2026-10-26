//! HTML attribute inference helpers.

use std::collections::VecDeque;
use std::sync::Arc;

use camino::Utf8Path;

use crate::types::*;

use super::collected::resolve_collected_type;
use super::substitute::{build_substitution, substitute_alias, substitute_type};
use super::{ResolutionContext, ResolveState};

/// `JSX.IntrinsicElements[element]` in React's own types at `react_file`, when this run has them: Full mode merges them,
/// curated mode indexes them apart (`ResolutionContext::react_index`), and `none` mode and the dependency loader don't
/// read them.
pub(super) fn intrinsic_element<'a>(
    element: &str,
    react_file: &Utf8Path,
    ctx: &'a ResolutionContext,
) -> Option<&'a CollectedType> {
    let elements = ctx.react_index().lookup_interface_exact(react_file, "React.JSX.IntrinsicElements")?;
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
                if let Some(iface) = ctx.react_index().lookup_interface_exact(react_file, &key) {
                    let params = ctx.react_index().lookup_interface_type_params_for(iface);
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
                } else if let Some(alias) = ctx.react_index().lookup_type_alias_exact(react_file, &key) {
                    let alias = match ctx.react_index().lookup_type_alias_params(react_file, &key) {
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

/// Every attribute name `JSX.IntrinsicElements[element]` declares in React's own types at `react_file`, in the order
/// `react_attribute` reads them; empty without those types.
fn react_attribute_names(element: &str, react_file: &Utf8Path, ctx: &ResolutionContext) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let Some(root) = intrinsic_element(element, react_file, ctx) else {
        return names;
    };
    let mut pending = VecDeque::from([root.clone()]);
    for _ in 0..64 {
        let Some(next) = pending.pop_front() else {
            break;
        };
        match next {
            CollectedType::AtFile { inner, .. } => pending.push_back(*inner),
            CollectedType::Intersection(members) => pending.extend(members),
            CollectedType::Named { name: type_name, args } => {
                let key = format!("React.{}", type_name.strip_prefix("React.").unwrap_or(&type_name));
                if let Some(iface) = ctx.react_index().lookup_interface_exact(react_file, &key) {
                    names.extend(iface.props.iter().map(|prop| prop.name.clone()));
                    pending.extend(iface.extends.iter().map(|extends| {
                        let (ExtendsRef::SameFile { name, type_args }
                        | ExtendsRef::Imported { local_name: name, type_args, .. }
                        | ExtendsRef::Builtin { name, type_args, .. }) = extends;
                        CollectedType::Named { name: name.clone(), args: type_args.clone() }
                    }));
                } else if let Some(alias) = ctx.react_index().lookup_type_alias_exact(react_file, &key) {
                    let alias = match ctx.react_index().lookup_type_alias_params(react_file, &key) {
                        Some(params) => {
                            substitute_alias(alias, &build_substitution(params, &args, react_file, &mut Vec::new()))
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
    let mut seen = rustc_hash::FxHashSet::default();
    names.retain(|name| seen.insert(name.clone()));
    names
}

/// The attributes of an HTML layer's element that `has` doesn't already list: every one `@types/react` declares when
/// `full`, else the curated subset (`react_types::is_curated_html_attr`), each as React declares it minus what the layer
/// omits. Without React's types the curated table stands in, typed by name.
pub(super) fn layer_attributes(
    layer: &InheritedLayer,
    full: bool,
    has: impl Fn(&str) -> bool,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Vec<Arc<ParsedProp>> {
    let Some(element) = layer.html_element.as_deref() else {
        return Vec::new();
    };
    let file = Utf8Path::new(&layer.file_name);
    // `ref` is a prop of `ComponentProps<"div">` (`ClassAttributes`) but not of `HTMLAttributes<HTMLDivElement>`.
    let bare = layer.type_name.strip_prefix("React.").unwrap_or(&layer.type_name);
    let takes_ref = matches!(bare, "ComponentProps" | "ComponentPropsWithRef");
    let key = format!("{}\0{element}\0{full}", layer.file_name);
    let cached = ctx.attribute_names.get(&key).map(|names| names.clone());
    let names: Arc<[String]> = match cached {
        Some(names) => names,
        None => {
            let declared = react_attribute_names(element, file, ctx);
            let names: Vec<String> = match (full, declared.is_empty()) {
                (true, false) => declared.into_iter().filter(|name| name != "key").collect(),
                (false, false) => declared
                    .into_iter()
                    .filter(|name| crate::react_types::is_curated_html_attr(element, name))
                    .collect(),
                (_, true) => Vec::new(),
            };
            let names: Arc<[String]> = names.into();
            ctx.attribute_names.insert(key, names.clone());
            names
        }
    };
    let loaded = !names.is_empty();
    if full && !loaded {
        state.diagnostics.push(
            Diagnostic::new(
                DiagnosticSeverity::Info,
                DiagnosticCode::MissingTypeLibrary,
                format!("@types/react could not be resolved, so the attributes of <{element}> are left out"),
            )
            .with_help("Check that @types/react is installed."),
        );
        return Vec::new();
    }
    let names: Vec<String> = if loaded {
        names.to_vec()
    } else {
        crate::react_types::notable_html_attrs(element)
            .iter()
            .chain(["ref"].iter())
            .map(|name| (*name).to_owned())
            .collect()
    };
    let parent = PropParent {
        name: format!("{}HTMLAttributes", capitalize_element(element)),
        file_name: "node_modules/@types/react/index.d.ts".to_owned(),
    };
    names
        .into_iter()
        .filter(|name| (takes_ref || name != "ref") && !has(name) && !layer.omitted.contains(name))
        .filter_map(|name| {
            if loaded {
                return super::alias::layer_attribute(layer, &name, ctx, state, depth).map(|prop| ctx.share(prop));
            }
            let prop_type = if name == "ref" {
                PropType::Named { name: "Ref".into(), args: vec![] }
            } else {
                infer_html_attr_prop_type(&name)
            };
            Some(ctx.share(ParsedProp::new(
                name,
                prop_type,
                false,
                None,
                String::new(),
                Default::default(),
                Some(parent.clone()),
                vec![],
            )))
        })
        .collect()
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
