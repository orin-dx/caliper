//! The type of a module-level `const` whose value spells its own type: a literal (`const SHOW_PARENT = 'SHOW_PARENT'`)
//! or an object literal (`const Icons = { success: …, error: … }`, read for its keys).
//!
//! Recorded as the alias `typeof NAME`, the name `record_declared_value_type` uses for a declared value, so `typeof
//! SHOW_PARENT` resolves to `"SHOW_PARENT"` and `keyof typeof Icons` to the union of its keys.

use oxc_ast::ast::*;

use crate::types::{CollectedObjectField, CollectedType, CollectedTypeAlias};

use super::{js_text, SourceDataCollector};

/// Objects with more keys than this are not a lookup table a prop's type is likely to name.
const MAX_KEYS: usize = 128;

/// The expression under `as const`, `satisfies T`, `!` and parentheses: none of them changes which literal it is.
fn peel<'a, 'b>(mut expr: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        expr = match expr {
            Expression::ParenthesizedExpression(inner) => &inner.expression,
            Expression::TSAsExpression(inner) => &inner.expression,
            Expression::TSSatisfiesExpression(inner) => &inner.expression,
            Expression::TSTypeAssertion(inner) => &inner.expression,
            Expression::TSNonNullExpression(inner) => &inner.expression,
            other => return other,
        };
    }
}

/// The literal type a literal expression has, as a `const` declares it.
fn literal(expr: &Expression<'_>) -> Option<CollectedType> {
    match peel(expr) {
        Expression::StringLiteral(s) => Some(CollectedType::StringLiteral(js_text::lossy(s.value).into_owned().into())),
        Expression::NumericLiteral(n) => Some(CollectedType::NumberLiteral(n.value)),
        Expression::BooleanLiteral(b) => Some(CollectedType::BoolLiteral(b.value)),
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => match peel(&u.argument) {
            Expression::NumericLiteral(n) => Some(CollectedType::NumberLiteral(-n.value)),
            _ => None,
        },
        _ => None,
    }
}

/// The keys of an object literal whose every member has a name that can be read without evaluating anything. A spread
/// or a computed key makes the key set unknown, so the object is not recorded.
fn static_fields(object: &ObjectExpression<'_>) -> Option<Vec<CollectedObjectField>> {
    if object.properties.is_empty() || object.properties.len() > MAX_KEYS {
        return None;
    }
    object
        .properties
        .iter()
        .map(|property| {
            let ObjectPropertyKind::ObjectProperty(property) = property else { return None };
            let name = match &property.key {
                PropertyKey::StaticIdentifier(id) => id.name.as_str().to_owned(),
                PropertyKey::StringLiteral(s) => js_text::lossy(s.value).into_owned(),
                _ => return None,
            };
            Some(CollectedObjectField {
                name,
                // The value's own type is not read: only the keys are.
                collected_type: literal(&property.value).unwrap_or(CollectedType::Unknown),
                required: true,
                description: String::new(),
                readonly: false,
                index_key: None,
            })
        })
        .collect()
}

impl SourceDataCollector<'_> {
    /// `const NAME = <literal | object literal>` at the top level of a module, as the alias `typeof NAME`.
    pub(super) fn record_const_value_type(&mut self, declarator: &VariableDeclarator<'_>) {
        if self.function_depth > 0 || !self.namespace_stack.is_empty() {
            return;
        }
        let (BindingPattern::BindingIdentifier(id), Some(init), None) =
            (&declarator.id, &declarator.init, &declarator.type_annotation)
        else {
            return;
        };
        let target = match (literal(init), peel(init)) {
            (Some(literal), _) => literal,
            (None, Expression::ObjectExpression(object)) => match static_fields(object) {
                Some(fields) => CollectedType::Object(fields),
                None => return,
            },
            _ => return,
        };
        let key = self.scoped_key(&format!("typeof {}", id.name));
        self.data
            .type_aliases
            .entry(key)
            .or_insert(CollectedTypeAlias::Passthrough { target, file_path: self.file_path.clone() });
    }
}
