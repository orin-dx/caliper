//! `export default Masonry as (<T = any>(props: PropsWithChildren<MasonryProps<T>> & RefAttributes<Ref>) => ReactElement)`:
//! a module that exports a component under a function type says in that type what its props are, so a props type the
//! declaration did not have (antd's Masonry adds `children` this way) is read from the assertion.

use oxc_ast::ast::*;

use crate::types::CollectedType;

use super::SourceDataCollector;

/// The function type an assertion's type is or contains: `(props: P) => E`, parenthesized or one member of `F & G`.
fn function_type<'a, 'b>(ty: &'b TSType<'a>) -> Option<&'b TSFunctionType<'a>> {
    match ty {
        TSType::TSFunctionType(function) => Some(function),
        TSType::TSParenthesizedType(inner) => function_type(&inner.type_annotation),
        TSType::TSIntersectionType(members) => members.types.iter().find_map(function_type),
        _ => None,
    }
}

impl SourceDataCollector<'_> {
    /// `export default Name as <function type>` over a component this file declares: the function's one props
    /// parameter becomes the component's props type.
    pub(super) fn record_asserted_props(&mut self, expr: &Expression<'_>) {
        let mut expr = expr;
        while let Expression::ParenthesizedExpression(inner) = expr {
            expr = &inner.expression;
        }
        let Expression::TSAsExpression(assertion) = expr else { return };
        let Expression::Identifier(id) = &assertion.expression else { return };
        let Some(function) = function_type(&assertion.type_annotation) else { return };
        let [parameter] = function.params.items.as_slice() else { return };
        let Some(annotation) = parameter.type_annotation.as_ref() else { return };
        let name = id.name.as_str();
        let file = self.file_path.clone();
        let Some(index) =
            self.data.component_mappings.iter().position(|m| m.is_binding_named(name) && m.file_path == file)
        else {
            return;
        };

        let type_params: Vec<crate::types::TypeName> = function
            .type_parameters
            .as_ref()
            .map(|declaration| declaration.params.iter().map(|param| param.name.name.as_str().into()).collect())
            .unwrap_or_default();
        let Some((props_name, mut props_args)) = self.extract_type_name_from_type(&annotation.type_annotation) else {
            return;
        };
        if crate::types::is_synthesized_alias(&props_name) && !type_params.is_empty() {
            self.data.type_alias_params.insert(self.scoped_key(&props_name), type_params.clone());
            props_args =
                type_params.iter().map(|name| CollectedType::Named { name: name.clone(), args: Vec::new() }).collect();
        }
        let mapping = &mut self.data.component_mappings[index];
        mapping.props_type_name = props_name;
        mapping.props_type_args = props_args;
        mapping.type_params = type_params;
    }
}
