//! One component's props from the checker: the export's call signature, its first parameter's type, that type's
//! properties.

use std::sync::Arc;

use caliper_docgen_core::types::ComponentExport;
use caliper_docgen_core::types::PropParent;
use caliper_docgen_core::types::semantic::{PropInterner, SemanticProp, unconverted_type};

use crate::Target;
use crate::convert::{Converter, PropKey, declared_in, without_undefined};
use crate::declared::Asked;
use crate::instantiate::{self, Instantiation};
use crate::rpc::{CallError, Rpc, SignatureKind, SymbolInfo, TypeInfo, flag};

pub(crate) enum Answer {
    Props(Props),
    /// The component keeps its structural result, for this reason.
    Fallback(String),
    /// A generic component, whose props are read from an instantiation of it ([`crate::instantiate`]).
    Instantiate(Instantiation),
}

/// What the checker says about one component's props type.
pub(crate) struct Props {
    pub(crate) props: Vec<Arc<SemanticProp>>,
    /// See [`caliper_docgen_core::types::semantic::SemanticComponent::composes`].
    pub(crate) composes: Option<Vec<String>>,
    /// See [`caliper_docgen_core::types::semantic::SemanticComponent::whole`].
    pub(crate) whole: Vec<PropParent>,
}

/// What `@types/react` declares (`key`, `ref`, `children` on `PropsWithChildren`, HTML attributes) and the project does
/// not. Reported with the flag set when the request asks for the name; what to do with them is the session's policy, not
/// ours. A property a project file also declares (`ComponentProps<'div'> & { className: 'mine' }`) is the component's
/// own and never React's. One a dependency declares again over React's (a library's `className`, its DOM handlers)
/// stays React's: the element layer stands for it.
fn is_react_declared(property: &SymbolInfo) -> bool {
    use caliper_docgen_core::origin::{Origin, declaration_origin};
    let files = || property.declarations.iter().map(|declaration| declared_in(declaration));
    files().any(|file| declaration_origin(file) == Origin::React) && files().all(|file| file.contains("/node_modules/"))
}

fn fallback(reason: &str) -> Result<Answer, CallError> {
    Ok(Answer::Fallback(reason.to_string()))
}

/// A module as the checker reads it: its own symbol, whose declaration names its file in the checker's spelling, and
/// its exports.
pub(crate) struct Module {
    pub(crate) symbol: SymbolInfo,
    pub(crate) exports: Vec<SymbolInfo>,
}

impl Module {
    /// Whether one of `symbol`'s declarations is in this module's file.
    fn declares(&self, symbol: &SymbolInfo) -> bool {
        let files: Vec<&str> = self.symbol.declarations.iter().map(|declaration| declared_in(declaration)).collect();
        symbol.declarations.iter().any(|declaration| files.contains(&declared_in(declaration)))
    }
}

/// The symbol `export` names in `module`: the export of exactly that name, through an alias to what it aliases, then
/// each property an attached sub-component is reached through. `Err` says why there is none, or why it is not the
/// component the extractor found: one declared in another file.
async fn exported_symbol(
    rpc: &Rpc,
    module: &Module,
    export: &ComponentExport,
) -> Result<Result<SymbolInfo, String>, CallError> {
    let Some(symbol) = module.exports.iter().find(|symbol| symbol.name == export.name) else {
        return Ok(Err(format!("the checker finds no export `{}` in its module", export.name)));
    };
    let mut symbol = if symbol.flags & flag::SYMBOL_ALIAS != 0 {
        rpc.aliased_symbol(&symbol.reference).await?
    } else {
        symbol.clone()
    };
    for member in &export.members {
        let parent = rpc.type_of_symbol(&symbol.reference).await?;
        let Some(property) = rpc.properties(parent.id).await?.into_iter().find(|property| property.name == *member)
        else {
            return Ok(Err(format!("the checker finds no property `{member}` on the export `{}`", export.name)));
        };
        symbol = property;
    }
    if !module.declares(&symbol) {
        return Ok(Err(format!("the export `{export}` is declared in another file")));
    }
    Ok(Ok(symbol))
}

/// The component's symbol: the first export the extractor recorded for it that the checker finds. Never another
/// export, such as the module's `default` when `default` is not one of them.
async fn component_symbol(
    rpc: &Rpc,
    module: &Module,
    target: &Target,
) -> Result<Result<(SymbolInfo, ComponentExport), String>, CallError> {
    let mut reasons = Vec::new();
    for export in &target.exports {
        match exported_symbol(rpc, module, export).await? {
            Ok(symbol) => return Ok(Ok((symbol, export.clone()))),
            Err(reason) => reasons.push(reason),
        }
    }
    Ok(Err(if reasons.is_empty() { "not exported by its module".to_string() } else { reasons.join("; ") }))
}

pub(crate) async fn props_of(
    rpc: &Rpc,
    converter: &Converter,
    interner: &PropInterner,
    module: &Module,
    target: &Target,
) -> Result<Answer, CallError> {
    let (symbol, export) = match component_symbol(rpc, module, target).await? {
        Ok(found) => found,
        Err(reason) => return Ok(Answer::Fallback(reason)),
    };

    let component = rpc.type_of_symbol(&symbol.reference).await?;
    let signatures = rpc.signatures(component.id, SignatureKind::Call).await?;
    let [signature] = signatures.as_slice() else {
        return fallback(if signatures.is_empty() {
            "no call signature (class or non-function component)"
        } else {
            "overloaded call signatures"
        });
    };
    let parameters = rpc.parameters(signature.id).await?;
    let Some(first) = parameters.first() else { return fallback("takes no props parameter") };
    if let Some(arguments) = instantiate::default_arguments(rpc, signature).await? {
        if !export.members.is_empty() {
            return fallback("a generic sub-component reached through its parent's property is not instantiated");
        }
        return Ok(Answer::Instantiate(Instantiation { export: export.name, arguments }));
    }

    let props_type = rpc.type_of_symbol(&first.reference).await?;
    Ok(props_of_type(rpc, converter, interner, &props_type, target).await?.map_or_else(Answer::Fallback, Answer::Props))
}

/// The props a component's props type holds, or why the component keeps its structural result.
pub(crate) async fn props_of_type(
    rpc: &Rpc,
    converter: &Converter,
    interner: &PropInterner,
    props_type: &TypeInfo,
    target: &Target,
) -> Result<Result<Props, String>, CallError> {
    if props_type.flags & (flag::ANY | flag::UNKNOWN | flag::UNION | flag::TYPE_PARAMETER) != 0 {
        return Ok(Err("props type is any, unknown, generic or a union".to_string()));
    }

    let own: Vec<SymbolInfo> = rpc
        .properties(props_type.id)
        .await?
        .into_iter()
        .filter(|property| {
            !is_react_declared(property)
                || target.react_wanted.as_ref().is_none_or(|wanted| wanted.contains(&property.name))
        })
        .collect();
    // Only a source the props type reaches whole can be left to a layer the answer adds (`Layering::kept` is asked with
    // them), so they are read before what is left to a layer.
    let (composes, whole) = converter.composes(props_type).await?;
    let whole = converter.as_parents(whole).await?;
    let (own, layered) = leave_to_layers(converter, interner, own, &whole, target).await?;
    let references: Vec<_> = own.iter().map(|property| &property.reference).collect();
    let types = if references.is_empty() { Vec::new() } else { converter.types_of_symbols(&references).await? };

    // React's own descriptions and tags are the structural pass's to give, and its props are most of a component's.
    let asked: Vec<Asked<'_>> = own
        .iter()
        .map(|property| {
            let react_declared = is_react_declared(property);
            Asked {
                property,
                description: !react_declared && !target.described.contains(&property.name),
                tags: !react_declared,
            }
        })
        .collect();
    let keys: Vec<PropKey> = asked
        .iter()
        .zip(&types)
        .map(|(asked, prop_type)| PropKey::of(asked.property, prop_type, asked.description, asked.tags))
        .collect();
    let mut props: Vec<Option<Arc<SemanticProp>>> = keys.iter().map(|key| converter.known_prop(key)).collect();
    let missing: Vec<usize> = (0..props.len()).filter(|index| props.get(*index).is_some_and(Option::is_none)).collect();
    let unknown_types: Vec<TypeInfo> =
        missing.iter().filter_map(|index| types.get(*index).map(|info| TypeInfo::clone(info))).collect();
    let converted = converter.convert_roots(&unknown_types).await?;
    let unknown_asked: Vec<Asked<'_>> = missing
        .iter()
        .filter_map(|index| asked.get(*index))
        .map(|asked| Asked { property: asked.property, description: asked.description, tags: asked.tags })
        .collect();
    let declared = converter.declared(&unknown_asked).await?;
    let mut keys: Vec<Option<PropKey>> = keys.into_iter().map(Some).collect();
    for ((index, prop_type), declared) in missing.iter().zip(converted).zip(declared) {
        let (Some(property), Some(key)) = (own.get(*index), keys.get_mut(*index).and_then(Option::take)) else {
            continue;
        };
        let required = property.flags & flag::SYMBOL_OPTIONAL == 0;
        let prop = interner.share(SemanticProp {
            name: property.name.clone(),
            prop_type: without_undefined(prop_type, !required),
            required,
            description: declared.description,
            tags: declared.tags,
            parent: declared.parent,
            declarations: declared.declarations,
            react_declared: is_react_declared(property),
        });
        converter.keep_prop(key, prop.clone());
        if let Some(slot) = props.get_mut(*index) {
            *slot = Some(prop);
        }
    }
    // In the props type's order: each converted prop in its place among those left to a layer. `props` has one slot per
    // converted prop, so a slot left empty above stays empty and shifts no other prop.
    let mut converted = props.into_iter();
    let props: Vec<Arc<SemanticProp>> =
        layered.into_iter().filter_map(|slot| slot.or_else(|| converted.next().flatten())).collect();
    Ok(Ok(Props { props, composes, whole }))
}

/// Of a component's props `own`, in order, those to convert, and a slot per prop of `own`: `Some` for one the merge
/// leaves to a collapsed layer ([`Layering::kept`](caliper_docgen_core::types::semantic::Layering::kept)), reported by
/// name and `parent` with [`unconverted_type`], `None` for one converted. Only where the prop is declared is asked for,
/// and only when the target has a layering under which the merge can leave a prop to a layer.
async fn leave_to_layers(
    converter: &Converter,
    interner: &PropInterner,
    own: Vec<SymbolInfo>,
    whole: &[PropParent],
    target: &Target,
) -> Result<(Vec<SymbolInfo>, Vec<Option<Arc<SemanticProp>>>), CallError> {
    // The merge leaves to a layer only a prop the structural answer does not list: when it lists every prop, where
    // they are declared is not asked first.
    let Some(layering) = target.layering.as_ref().filter(|layering| {
        layering.may_layer()
            && own.iter().any(|property| !is_react_declared(property) && !layering.lists(&property.name))
    }) else {
        let slots = vec![None; own.len()];
        return Ok((own, slots));
    };
    let asked: Vec<Asked<'_>> =
        own.iter().map(|property| Asked { property, description: false, tags: false }).collect();
    let declared = converter.declared(&asked).await?;
    let answer: Vec<(&str, Option<&PropParent>, bool)> = own
        .iter()
        .zip(&declared)
        .map(|(property, declared)| (property.name.as_str(), declared.parent.as_ref(), is_react_declared(property)))
        .collect();
    let kept = layering.kept(&answer, whole);
    if kept.iter().all(|kept| *kept) {
        let slots = vec![None; own.len()];
        return Ok((own, slots));
    }
    let mut converting = Vec::with_capacity(own.len());
    let mut slots = Vec::with_capacity(own.len());
    for ((property, declared), kept) in own.into_iter().zip(declared).zip(kept) {
        if kept {
            converting.push(property);
            slots.push(None);
            continue;
        }
        let required = property.flags & flag::SYMBOL_OPTIONAL == 0;
        slots.push(Some(interner.share(SemanticProp {
            prop_type: unconverted_type(),
            required,
            description: String::new(),
            tags: Default::default(),
            parent: declared.parent,
            declarations: declared.declarations,
            react_declared: false,
            name: property.name,
        })));
    }
    Ok((converting, slots))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(declaration: &str) -> SymbolInfo {
        SymbolInfo {
            reference: serde_json::Value::Null.into(),
            name: "x".to_string(),
            flags: 4,
            declarations: vec![declaration.to_string()],
            parent: None,
        }
    }

    #[test]
    fn the_declaring_path_is_what_follows_the_node_id_and_position() {
        assert_eq!(
            declared_in("48.264./proj/node_modules/@types/react/index.d.ts"),
            "/proj/node_modules/@types/react/index.d.ts"
        );
        assert_eq!(declared_in("garbage"), "");
    }

    #[test]
    fn only_properties_declared_in_types_react_are_excluded() {
        assert!(is_react_declared(&property("1.2./p/node_modules/@types/react/index.d.ts")));
        assert!(!is_react_declared(&property("1.2./p/src/button.tsx")));
        assert!(!is_react_declared(&property("1.2./p/node_modules/@chakra-ui/react/dist/index.d.ts")));
    }
}
