//! The props a factory call gives the component it builds, read from the call's own arguments (ADR 0028, F1).
//!
//! `const Center = chakra("div", { variants: { inline: { true: {...} } } })` takes `inline` from its style recipe, and
//! `const Divider = createLeafComponent(Node, function Divider({size}: {size?: Size}, ref) {...})` takes the props type
//! its render function annotates. The structural reader resolves such a component through the props type the detection
//! chose (the one declared for the tag, or the callee's result), which hides these. Nothing here names a library and
//! nothing here decides what a component is: the props are recorded for every call bound to a PascalCase name, and
//! attached only to a mapping a detection rule built from that same declaration. `ref`, which such factories add
//! through their result type, is not invented here: the call's arguments do not say it.

use oxc_ast::ast::*;

use crate::types::{CollectedObjectField, CollectedType, RawDefault, TypeName};

use super::SourceDataCollector;
use super::component::peel_expr;
use super::evidence::{FunctionShape, renders};

/// The own props a factory call's arguments give, waiting for detection to decide whether the call is a component.
pub(crate) struct PendingFactoryProps {
    /// Where the declaration starts: the mapping a rule builds from it starts there too.
    span_start: u32,
    props: CollectedType,
    /// Type parameters of the render function, which its props type names as placeholders.
    type_params: Vec<TypeName>,
    /// The defaults the render function's destructured props parameter gives, read as for a function component.
    defaults: rustc_hash::FxHashMap<String, RawDefault>,
}

impl<'src> SourceDataCollector<'src> {
    /// Record the own props `init`, the initializer of the PascalCase binding declared at `span_start`, gives the
    /// component it builds, when it is a call: (a) the type annotation of the first parameter of a render function given as
    /// its last argument (one that renders, read as for a function component; any other callback is ignored); (b) otherwise, the keys of an object literal `variants`
    /// inside an object literal argument, when that is a style recipe.
    pub(super) fn record_factory_props<'a>(&mut self, span_start: u32, init: &Expression<'a>) {
        let Expression::CallExpression(call) = peel_expr(init) else { return };
        let annotated = call.arguments.last().and_then(Argument::as_expression).map(peel_expr).and_then(|last| {
            let (params, type_parameters, shape, asynchronous) = match last {
                Expression::ArrowFunctionExpression(f) => {
                    (&f.params, f.type_parameters.as_deref(), FunctionShape::of_arrow(f), f.r#async)
                }
                Expression::FunctionExpression(f) => {
                    (&f.params, f.type_parameters.as_deref(), FunctionShape::of_function(f), f.r#async)
                }
                _ => return None,
            };
            // An allow-list: the callback is the render function only when it is proven to be one. Its first
            // parameter is a plain or destructured props parameter, and it renders (`evidence::renders`: it returns
            // JSX or an element, not a string, a function or nothing). An async or generator function does not render.
            let first = params.items.first()?;
            if asynchronous
                || !matches!(first.pattern, BindingPattern::BindingIdentifier(_) | BindingPattern::ObjectPattern(_))
                || renders(&shape, &*self).is_err()
            {
                return None;
            }
            let annotation = first.type_annotation.as_ref()?;
            Some((&annotation.type_annotation, type_parameters, self.extract_param_defaults(params)))
        });
        let (props, type_params, defaults) = if let Some((annotation, type_parameters, defaults)) = annotated {
            (self.ts_type_to_collected(annotation), super::type_param_names(type_parameters), defaults)
        } else if let Some(variants) = call.arguments.iter().find_map(recipe_variants) {
            (CollectedType::Object(variants), Vec::new(), Default::default())
        } else {
            return;
        };
        self.pending_factory_props.push(PendingFactoryProps { span_start, props, type_params, defaults });
    }

    /// Give each mapping a detection rule built from a factory call the own props recorded for that call. A mapping
    /// that already reads its props from a function's parameter (`forwardRef`, `memo`) records none: the rules that
    /// unwrap those do not reach `record_factory_props`.
    pub(super) fn attach_factory_props(&mut self) {
        for pending in std::mem::take(&mut self.pending_factory_props) {
            for mapping in self.data.component_mappings.iter_mut().filter(|m| m.span_start == pending.span_start) {
                mapping.factory_type_params.clone_from(&pending.type_params);
                mapping.factory_props = Some(pending.props.clone());
                for (name, default) in &pending.defaults {
                    mapping.param_defaults.entry(name.clone()).or_insert_with(|| default.clone());
                }
            }
        }
    }
}

/// The variant props of a style recipe: `{ base, variants: { inline: { true: {...} }, size: { sm: {...} } } }` gives
/// `inline?: boolean` and `size?: "sm"`. A recipe's variant maps each option to a style object, so a `variants` whose
/// members are not all objects of objects (an animation's `{ visible: { opacity: 1 } }`) is not one, and gives nothing.
fn recipe_variants(argument: &Argument<'_>) -> Option<Vec<CollectedObjectField>> {
    let Expression::ObjectExpression(config) = peel_expr(argument.as_expression()?) else { return None };
    let variants = config.properties.iter().find_map(|property| match property {
        ObjectPropertyKind::ObjectProperty(p) if static_key(&p.key).as_deref() == Some("variants") => {
            match peel_expr(&p.value) {
                Expression::ObjectExpression(variants) => Some(variants),
                _ => None,
            }
        }
        _ => None,
    })?;
    let mut fields = Vec::new();
    for property in &variants.properties {
        // A spread variant set (`...base.variants`) cannot be read here; the keys written beside it still can.
        let ObjectPropertyKind::ObjectProperty(variant) = property else { continue };
        // A computed key or a variant held elsewhere (`size: sizeVariants`) cannot be read here; the variants written
        // beside it still can.
        let Some(name) = static_key(&variant.key) else { continue };
        let Expression::ObjectExpression(options) = peel_expr(&variant.value) else { continue };
        let mut members = Vec::new();
        let mut boolean = false;
        for option in &options.properties {
            let ObjectPropertyKind::ObjectProperty(option) = option else { continue };
            if !matches!(peel_expr(&option.value), Expression::ObjectExpression(_)) {
                return None;
            }
            match static_key(&option.key)?.as_str() {
                "true" | "false" => boolean = true,
                key => members.push(CollectedType::StringLiteral(key.into())),
            }
        }
        if boolean {
            members.push(CollectedType::Boolean);
        }
        let collected_type = match members.len() {
            0 => continue,
            1 => members.remove(0),
            _ => CollectedType::Union(members),
        };
        fields.push(CollectedObjectField {
            name,
            collected_type,
            required: false,
            description: String::new(),
            readonly: false,
            index_key: None,
            tags: Default::default(),
        });
    }
    (!fields.is_empty()).then_some(fields)
}

/// A property key written as a name, a string or a number; `None` for a computed key.
fn static_key(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str().to_owned()),
        PropertyKey::StringLiteral(s) => Some(super::js_text::lossy(s.value).into_owned()),
        PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
