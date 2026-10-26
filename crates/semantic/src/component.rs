//! One component's props from the checker: the export's call signature, its first parameter's type, that type's
//! properties.

use std::sync::Arc;

use caliper_docgen_core::types::semantic::{PropInterner, SemanticProp};

use crate::convert::{without_undefined, Converter};
use crate::rpc::{flag, CallError, Rpc, SymbolInfo};
use crate::Target;

pub(crate) enum Answer {
    Props(Vec<Arc<SemanticProp>>),
    /// The component keeps its structural result, for this reason.
    Fallback(String),
}

/// `declarations` entries read `"<nodeId>.<pos>.<path>"`.
fn declared_in(declaration: &str) -> &str {
    declaration.splitn(3, '.').nth(2).unwrap_or("")
}

/// What `@types/react` declares (`key`, `ref`, `children` on `PropsWithChildren`, HTML attributes). Reported with the
/// flag set when the request asks for the name; what to do with them is the session's policy, not ours.
fn is_react_declared(property: &SymbolInfo) -> bool {
    property.declarations.iter().any(|declaration| declared_in(declaration).contains("/@types/react/"))
}

/// The export names a component may have. A display name is not always the export: Mantine sets
/// `displayName = "@mantine/core/Accordion"` on the export `Accordion`, and a static one reads `Menu.Item`.
fn candidate_names(display_name: &str) -> Vec<&str> {
    let mut names = vec![display_name];
    for separator in ['/', '.'] {
        if let Some((_, last)) = display_name.rsplit_once(separator) {
            if !last.is_empty() && !names.contains(&last) {
                names.push(last);
            }
        }
    }
    names
}

fn fallback(reason: &str) -> Result<Answer, CallError> {
    Ok(Answer::Fallback(reason.to_string()))
}

pub(crate) async fn props_of(
    rpc: &Rpc,
    converter: &Converter,
    interner: &PropInterner,
    exports: &[SymbolInfo],
    target: &Target,
) -> Result<Answer, CallError> {
    let display_name = target.display_name.as_str();
    let export = candidate_names(display_name)
        .into_iter()
        .find_map(|name| exports.iter().find(|symbol| symbol.name == name))
        .or_else(|| exports.iter().find(|symbol| symbol.name == "default"));
    let Some(export) = export else { return fallback("not found among the module's exports") };

    let mut reference = export.reference.clone();
    if export.flags & flag::SYMBOL_ALIAS != 0 {
        reference = rpc.aliased_symbol(&reference).await?.reference;
    }

    let component = rpc.type_of_symbol(&reference).await?;
    let signatures = rpc.signatures(component.id).await?;
    let [signature] = signatures.as_slice() else {
        return fallback(if signatures.is_empty() {
            "no call signature (class or non-function component)"
        } else {
            "overloaded call signatures"
        });
    };
    let parameters = rpc.parameters(signature.id).await?;
    let Some(first) = parameters.first() else { return fallback("takes no props parameter") };

    let props_type = rpc.type_of_symbol(&first.reference).await?;
    if props_type.flags & (flag::ANY | flag::UNKNOWN | flag::UNION | flag::TYPE_PARAMETER) != 0 {
        return fallback("props type is any, unknown, generic or a union");
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
    let references: Vec<_> = own.iter().map(|property| &property.reference).collect();
    let types = if references.is_empty() { Vec::new() } else { rpc.types_of_symbols(&references).await? };

    let mut props = Vec::with_capacity(own.len());
    for (property, ty) in own.iter().zip(&types) {
        let required = property.flags & flag::SYMBOL_OPTIONAL == 0;
        let react_declared = is_react_declared(property);
        props.push(interner.share(SemanticProp {
            name: property.name.clone(),
            prop_type: without_undefined(converter.convert(ty, 0).await?, !required),
            required,
            react_declared,
            // React's own descriptions are the structural pass's to give, and fetching one per attribute is a call each.
            description: if react_declared || target.described.contains(&property.name) {
                String::new()
            } else {
                rpc.documentation(&property.reference).await?
            },
        }));
    }
    Ok(Answer::Props(props))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(declaration: &str) -> SymbolInfo {
        SymbolInfo {
            reference: serde_json::Value::Null,
            name: "x".to_string(),
            flags: 4,
            declarations: vec![declaration.to_string()],
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
    fn a_display_name_with_a_package_or_namespace_prefix_also_tries_its_last_segment() {
        assert_eq!(candidate_names("Button"), ["Button"]);
        assert_eq!(candidate_names("@mantine/core/Accordion"), ["@mantine/core/Accordion", "Accordion"]);
        assert_eq!(candidate_names("Menu.Item"), ["Menu.Item", "Item"]);
    }

    #[test]
    fn only_properties_declared_in_types_react_are_excluded() {
        assert!(is_react_declared(&property("1.2./p/node_modules/@types/react/index.d.ts")));
        assert!(!is_react_declared(&property("1.2./p/src/button.tsx")));
        assert!(!is_react_declared(&property("1.2./p/node_modules/@chakra-ui/react/dist/index.d.ts")));
    }
}
