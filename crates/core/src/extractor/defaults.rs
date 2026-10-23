//! Default value extraction from destructured function parameters.

use oxc_ast::ast::*;

use crate::types::{DefaultSource, RawDefault};

use super::SourceDataCollector;

impl<'src> SourceDataCollector<'src> {
    /// Extract default values from destructured function parameters.
    ///
    /// Handles `({ size = 'md', disabled = false }: Props) => ...` patterns.
    pub(super) fn extract_param_defaults<'a>(
        &self,
        params: &FormalParameters<'a>,
    ) -> rustc_hash::FxHashMap<String, RawDefault> {
        let mut defaults = rustc_hash::FxHashMap::default();

        let Some(first_param) = params.items.first() else {
            return defaults;
        };
        // In OXC 0.135, BindingPattern is an enum directly (not a struct with .kind)
        let BindingPattern::ObjectPattern(obj) = &first_param.pattern else {
            return defaults;
        };

        for prop in &obj.properties {
            // In OXC 0.135, BindingProperty.value is BindingPattern.
            // A default value `{ size = 'md' }` is represented as
            // BindingProperty { key: "size", value: BindingPattern::AssignmentPattern { left: "size", right: 'md' } }
            let (name, default_expr) = match &prop.value {
                BindingPattern::AssignmentPattern(ap) => {
                    let name = match &ap.left {
                        BindingPattern::BindingIdentifier(id) => id.name.as_str().to_owned(),
                        _ => continue,
                    };
                    (name, &ap.right)
                }
                _ => continue,
            };
            let raw_default = self.eval_expr_as_default(default_expr, DefaultSource::Destructuring);
            defaults.insert(name, raw_default);
        }
        defaults
    }

    pub(super) fn eval_expr_as_default<'a>(&self, expr: &Expression<'a>, source: DefaultSource) -> RawDefault {
        match expr {
            Expression::NumericLiteral(n) => RawDefault { value: n.value.to_string(), computed: false, source },
            Expression::BooleanLiteral(b) => RawDefault { value: b.value.to_string(), computed: false, source },
            Expression::NullLiteral(_) => RawDefault { value: "null".into(), computed: false, source },
            Expression::Identifier(id) if id.name.as_str() == "undefined" => {
                RawDefault { value: "undefined".into(), computed: false, source }
            }
            // String, array and object literals keep their source text, quotes included, as react-docgen-typescript and
            // Storybook's React docgen print them.
            Expression::StringLiteral(_) | Expression::ArrayExpression(_) | Expression::ObjectExpression(_) => {
                use oxc_span::GetSpan;
                let span = expr.span();
                RawDefault {
                    value: self.source[span.start as usize..span.end as usize].to_owned(),
                    computed: false,
                    source,
                }
            }
            // Everything else (identifier refs, calls, ternaries): computed
            _ => {
                use oxc_span::GetSpan;
                let span = expr.span();
                RawDefault {
                    value: self.source[span.start as usize..span.end as usize].to_owned(),
                    computed: true,
                    source,
                }
            }
        }
    }
}
