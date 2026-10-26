//! Interface-adjacent collection: const enums, cva variants, displayName scanning.

use oxc_ast::ast::*;
use rustc_hash::FxHashMap;

use crate::types::{ComponentMapping, DefaultSource, DetectionRule, DiagnosticCode, EnumEntry, EnumValue, RawDefault};

use super::{is_pascal_case, SourceDataCollector};

/// Extract the binding name and the `as const`-peeled inner expression from
/// `const NAME = <expr> as const` (or `<const><expr>`). Shared by
/// `try_collect_const_enum` (object) and `try_collect_const_array` (array) —
/// the only difference between them is what kind of expression they expect
/// once the cast is peeled off.
fn as_const_literal<'a, 'b>(decl: &'b VariableDeclarator<'a>) -> Option<(std::string::String, &'b Expression<'a>)> {
    let BindingPattern::BindingIdentifier(id) = &decl.id else { return None };
    let init = decl.init.as_ref()?;
    let inner = match init {
        Expression::TSAsExpression(tsa) => &tsa.expression,
        Expression::TSTypeAssertion(ta) => &ta.expression,
        _ => return None,
    };
    Some((id.name.as_str().to_owned(), inner))
}

/// The value a literal type stands for: `"a"`, `1`, `-1`, `true`.
fn literal_member(ty: &TSType<'_>) -> Option<EnumValue> {
    let TSType::TSLiteralType(lit) = ty else { return None };
    match &lit.literal {
        TSLiteral::StringLiteral(s) => Some(EnumValue::String(s.value.as_str().to_owned())),
        TSLiteral::NumericLiteral(n) => Some(EnumValue::Number(n.value)),
        TSLiteral::BooleanLiteral(b) => Some(EnumValue::Bool(b.value)),
        TSLiteral::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => match &u.argument {
            Expression::NumericLiteral(n) => Some(EnumValue::Number(-n.value)),
            _ => None,
        },
        _ => None,
    }
}

impl<'src> SourceDataCollector<'src> {
    // ─── `const` enum (as-const objects) collection ──────────────────────────

    pub(super) fn try_collect_const_enum<'a>(&mut self, decl: &VariableDeclarator<'a>) {
        let Some((name, obj_expr)) = as_const_literal(decl) else { return };

        let obj = match obj_expr {
            Expression::ObjectExpression(o) => o,
            _ => return,
        };

        let entries: Vec<EnumEntry> = obj
            .properties
            .iter()
            .filter_map(|prop| match prop {
                ObjectPropertyKind::ObjectProperty(op) => {
                    let key = op.key.static_name()?.to_string();
                    let value = self.expression_to_enum_value(&op.value)?;
                    let desc = self.find_jsdoc(op.span.start);
                    Some(EnumEntry { name: key, value, description: desc })
                }
                _ => None,
            })
            .collect();

        if !entries.is_empty() {
            let key = self.scoped_key(&name);
            self.data.enums.insert(key, entries);
        }
    }

    // ─── `const` array (as-const arrays) collection ───────────────────────────

    /// Detect: `const X = [...] as const` — a flat array literal, referenced via
    /// `(typeof X)[number]` to build a literal union without an explicit `enum`
    /// (e.g. antd's `type ButtonType = (typeof _ButtonTypes)[number]`). Distinct
    /// from `try_collect_const_enum` above (object, not array) and stored
    /// separately in `SourceData::const_arrays` — see that field's doc comment
    /// for why it isn't folded into `enums`.
    pub(super) fn try_collect_const_array<'a>(&mut self, decl: &VariableDeclarator<'a>) {
        let Some((name, arr_expr)) = as_const_literal(decl) else {
            self.try_collect_annotated_tuple(decl);
            return;
        };

        let arr = match arr_expr {
            Expression::ArrayExpression(a) => a,
            _ => return,
        };

        // A spread of a named array (`...PresetColors`) is kept by name; the resolver finds the array it names.
        let values: Vec<EnumValue> = arr
            .elements
            .iter()
            .filter_map(|el| match el {
                ArrayExpressionElement::SpreadElement(spread) => match &spread.argument {
                    Expression::Identifier(id) => Some(EnumValue::Spread { spread: id.name.as_str().to_owned() }),
                    _ => None,
                },
                other => other.as_expression().and_then(|e| self.expression_to_enum_value(e)),
            })
            .collect();

        if !values.is_empty() {
            let key = self.scoped_key(&name);
            self.data.const_arrays.insert(key, values);
        }
    }

    /// Detect: `declare const X: readonly ["a", "b"]` — the same array as `const X = ["a", "b"] as const`, declared by its
    /// type instead of its value (what a `.d.ts` says, e.g. Radix's `ALIGN_OPTIONS`). TypeScript reads `typeof X` as the
    /// declared type of the symbol and `[number]` as the union of the tuple's elements, so the members go where an
    /// initializer's would. Only a tuple of literals is kept: any other element leaves the const out, as an array
    /// literal with a computed element would.
    fn try_collect_annotated_tuple<'a>(&mut self, decl: &VariableDeclarator<'a>) {
        let BindingPattern::BindingIdentifier(id) = &decl.id else { return };
        let Some(annotation) = decl.type_annotation.as_ref() else { return };
        let mut ty = &annotation.type_annotation;
        while let TSType::TSTypeOperatorType(op) = ty {
            if op.operator != TSTypeOperatorOperator::Readonly {
                return;
            }
            ty = &op.type_annotation;
        }
        let TSType::TSTupleType(tuple) = ty else { return };
        let values: Option<Vec<EnumValue>> =
            tuple.element_types.iter().map(|element| element.as_ts_type().and_then(literal_member)).collect();
        let Some(values) = values.filter(|values| !values.is_empty()) else { return };
        let key = self.scoped_key(id.name.as_str());
        self.data.const_arrays.insert(key, values);
    }

    /// Detect a call with a literal `variants` config, such as `cva(base, { variants })` or `chakra("div", { variants })`.
    /// Store each variant key's values in self.data.enums under the scoped key.
    pub(super) fn try_collect_cva_call<'a>(&mut self, decl: &VariableDeclarator<'a>, name: &str) {
        let Some(init) = decl.init.as_ref() else { return };
        let call = match init {
            Expression::CallExpression(ce) => ce,
            _ => return,
        };

        let callee_name = match &call.callee {
            Expression::Identifier(id) => id.name.as_str(),
            Expression::StaticMemberExpression(m) => m.property.name.as_str(),
            _ => return,
        };

        if !matches!(callee_name, "cva" | "tv" | "defineRecipe" | "recipe" | "defineSlotRecipe" | "chakra") {
            return;
        }

        // cva(baseClasses, { variants }) and chakra(element, { variants }) take config at index 1.
        let arg_index = usize::from(matches!(callee_name, "cva" | "chakra"));
        let Some(second_arg) = call.arguments.get(arg_index) else { return };
        let second_expr = match second_arg {
            Argument::SpreadElement(_) => return,
            other => match other.as_expression() {
                Some(e) => e,
                None => return,
            },
        };
        let obj = match second_expr {
            Expression::ObjectExpression(o) => o,
            _ => return,
        };

        let variants_value = obj.properties.iter().find_map(|prop| {
            if let ObjectPropertyKind::ObjectProperty(p) = prop {
                if let PropertyKey::StaticIdentifier(key) = &p.key {
                    if key.name.as_str() == "variants" {
                        return match &p.value {
                            Expression::ObjectExpression(o) => Some(o),
                            _ => None,
                        };
                    }
                }
            }
            None
        });
        let Some(variants_value) = variants_value else { return };

        let scoped_key = self.scoped_key(name);
        let mut entries: Vec<EnumEntry> = Vec::new();

        for prop in &variants_value.properties {
            if let ObjectPropertyKind::ObjectProperty(variant_prop) = prop {
                let variant_key = match &variant_prop.key {
                    PropertyKey::StaticIdentifier(id) => id.name.as_str().to_owned(),
                    PropertyKey::StringLiteral(s) => s.value.as_str().to_owned(),
                    _ => continue,
                };

                // The value is another object: { default: "...", sm: "...", ... }
                let values_obj = match &variant_prop.value {
                    Expression::ObjectExpression(o) => o,
                    _ => continue,
                };

                for value_prop in &values_obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(vp) = value_prop {
                        let value_name = match &vp.key {
                            PropertyKey::StaticIdentifier(id) => id.name.as_str().to_owned(),
                            PropertyKey::StringLiteral(s) => s.value.as_str().to_owned(),
                            _ => continue,
                        };

                        entries.push(EnumEntry {
                            name: variant_key.clone(),
                            value: EnumValue::String(value_name),
                            description: String::new(),
                        });
                    }
                }
            }
        }

        if !entries.is_empty() {
            self.data.enums.insert(scoped_key, entries);
        }
    }

    pub(super) fn expression_to_enum_value<'a>(&self, expr: &Expression<'a>) -> Option<EnumValue> {
        match expr {
            Expression::StringLiteral(s) => Some(EnumValue::String(s.value.as_str().to_owned())),
            Expression::NumericLiteral(n) => Some(EnumValue::Number(n.value)),
            Expression::BooleanLiteral(b) => Some(EnumValue::Bool(b.value)),
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => match &u.argument {
                Expression::NumericLiteral(n) => Some(EnumValue::Number(-n.value)),
                _ => None,
            },
            _ => None,
        }
    }

    // ─── Static member assignment scanning ────────────────────────────────────

    /// Parse `X.Y = <right>` out of an `ExpressionStatement`. Shared by every
    /// scan below that looks for a specific static property name being
    /// assigned on an already-detected component's binding.
    fn static_member_assignment<'a, 'b>(
        &self,
        stmt: &'b ExpressionStatement<'a>,
    ) -> Option<(std::string::String, &'b str, &'b Expression<'a>)> {
        let Expression::AssignmentExpression(assign) = &stmt.expression else { return None };
        let AssignmentTarget::StaticMemberExpression(sme) = &assign.left else { return None };
        let obj = self.expression_to_ident_name(&sme.object);
        Some((obj, sme.property.name.as_str(), &assign.right))
    }

    // ─── `displayName` scanning ───────────────────────────────────────────────

    /// Scan `Button.displayName = "Button"` and record the rename to apply
    /// once the whole file has been scanned (see `pending_display_name_renames`'s
    /// doc comment on why this can't be applied immediately).
    pub(super) fn try_scan_display_name<'a>(&mut self, stmt: &ExpressionStatement<'a>) {
        let Some((obj_name, "displayName", right)) = self.static_member_assignment(stmt) else { return };
        let Expression::StringLiteral(s) = right else { return };
        self.pending_display_name_renames.push((obj_name, s.value.as_str().to_owned()));
    }

    // ─── `defaultProps` scanning ───────────────────────────────────────────────

    /// Scan `Button.defaultProps = { size: 'md' }` and merge into the matching
    /// component mapping's `param_defaults` — the same field destructured
    /// defaults (`function Button({ size = 'md' })`) populate, via
    /// `extract_param_defaults`. Deprecated in React 19 but still shipped in
    /// real .d.ts/.tsx (MUI, among others).
    pub(super) fn try_scan_default_props<'a>(&mut self, stmt: &ExpressionStatement<'a>) {
        let Some((obj_name, "defaultProps", right)) = self.static_member_assignment(stmt) else { return };
        let Expression::ObjectExpression(defaults_obj) = right else { return };

        let defaults: FxHashMap<std::string::String, RawDefault> = defaults_obj
            .properties
            .iter()
            .filter_map(|prop| match prop {
                ObjectPropertyKind::ObjectProperty(op) => {
                    let key = op.key.static_name()?.to_string();
                    let raw_default = self.eval_expr_as_default(&op.value, DefaultSource::DefaultProps);
                    Some((key, raw_default))
                }
                _ => None,
            })
            .collect();

        if defaults.is_empty() {
            return;
        }

        if let Some(mapping) = self.data.component_mappings.iter_mut().find(|m| m.component_name == obj_name) {
            mapping.param_defaults.extend(defaults);
        }
    }

    // ─── `Object.assign(Parent, { Sub: ... })` inline-only sub-components ────

    /// Detect `Object.assign(Parent, { Sub: function(...) {...}, ... })` and
    /// synthesize a component mapping for each inline function/arrow-function
    /// value whose key is PascalCase — the one sub-case of this pattern that
    /// isn't already covered: a sub-component that exists ONLY inside the
    /// object-literal argument, with no independent top-level binding of its
    /// own. A bare identifier reference (`{ Icon: IconImpl }`) is
    /// deliberately left alone: `IconImpl` is independently declared and
    /// already detected under its own name via Pattern 4/HOC, and
    /// `rdt-coverage.md`'s "compound components" investigation found no
    /// real-world case needing anything more than that — cosmetic
    /// dot-qualified naming (`"Parent.Sub"`) was considered and explicitly
    /// declined there as speculative.
    pub(super) fn try_scan_object_assign_sub_components<'a>(&mut self, stmt: &ExpressionStatement<'a>) {
        let Expression::CallExpression(call) = &stmt.expression else { return };
        let Expression::StaticMemberExpression(callee) = &call.callee else { return };
        if !matches!(&callee.object, Expression::Identifier(id) if id.name.as_str() == "Object") {
            return;
        }
        if callee.property.name.as_str() != "assign" {
            return;
        }
        let Some(Argument::Identifier(parent_id)) = call.arguments.first() else { return };
        if !is_pascal_case(parent_id.name.as_str()) {
            return;
        }
        let Some(second_arg) = call.arguments.get(1) else { return };
        let Some(Expression::ObjectExpression(obj)) = second_arg.as_expression() else { return };

        for prop in &obj.properties {
            let ObjectPropertyKind::ObjectProperty(op) = prop else { continue };
            let Some(key_name) = op.key.static_name().map(|n| n.to_string()) else { continue };
            if !is_pascal_case(&key_name) {
                continue;
            }

            // Only inline function/arrow expressions carry a sub-component that
            // would otherwise be entirely unreachable; a bare identifier
            // reference is already independently detected under its own name.
            let (params, span_start, span_end) = match &op.value {
                Expression::FunctionExpression(fe) => (&fe.params, fe.span.start, fe.span.end),
                Expression::ArrowFunctionExpression(afe) => (&afe.params, afe.span.start, afe.span.end),
                Expression::Identifier(sub) => {
                    self.record_attached(sub.name.as_str(), parent_id.name.as_str());
                    continue;
                }
                _ => continue,
            };
            self.record_attached(&key_name, parent_id.name.as_str());

            let Some(first_param) = params.items.first() else { continue };
            let Some(type_ann) = &first_param.type_annotation else {
                self.record_skip(
                    DiagnosticCode::SkippedCandidate,
                    format!("'{key_name}' is an Object.assign-attached sub-component with an untyped first param"),
                    first_param.span,
                );
                continue;
            };
            let Some((props_name, type_args)) = self.extract_type_name_from_type(&type_ann.type_annotation) else {
                self.record_skip(
                    DiagnosticCode::SkippedCandidate,
                    format!(
                        "'{key_name}' is an Object.assign-attached sub-component whose first param's type \
                         annotation isn't a recognizable props type reference"
                    ),
                    type_ann.span,
                );
                continue;
            };

            let (description, tags) = self.find_jsdoc_with_tags(span_start);
            let param_defaults = self.extract_param_defaults(params);
            self.data.component_mappings.push(
                ComponentMapping::new(
                    key_name,
                    props_name,
                    self.file_path.clone(),
                    span_start,
                    span_end,
                    DetectionRule::AttachedSubComponent,
                )
                .with_props_type_args(type_args)
                .with_doc(description, tags)
                .with_param_defaults(param_defaults),
            );
        }
    }
}
