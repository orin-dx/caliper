//! `impl Visit for SourceDataCollector` — the AST walker entry points.

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_syntax::scope::ScopeFlags;

use crate::types::{
    CollectedInterface, ComponentMapping, DetectionRule, DiagnosticCode, EnumEntry, EnumValue, ExtendsRef,
    ImportBinding, LexedExport, RawProp, TypeName,
};

use super::component::Binding;
use super::{declaration_name, is_pascal_case, SourceDataCollector};

impl<'a, 'src> Visit<'a> for SourceDataCollector<'src> {
    fn visit_import_declaration(&mut self, node: &ImportDeclaration<'a>) {
        let specifier = node.source.value.as_str().to_owned();
        let is_type_only = node.import_kind.is_type();

        if let Some(specifiers) = &node.specifiers {
            for spec in specifiers {
                match spec {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => {
                        let local = s.local.name.as_str();
                        let imported = s.imported.name();
                        let imported_str = imported.as_str();
                        self.imported_names.insert(local.into());
                        self.data.imports.push(ImportBinding {
                            local_name: local.into(),
                            exported_name: imported_str.into(),
                            specifier: specifier.clone(),
                            is_type_only: is_type_only || s.import_kind.is_type(),
                        });
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        let local = s.local.name.as_str();
                        self.imported_names.insert(local.into());
                        self.data.imports.push(ImportBinding {
                            local_name: local.into(),
                            exported_name: "default".into(),
                            specifier: specifier.clone(),
                            is_type_only,
                        });
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        let local = s.local.name.as_str();
                        self.imported_names.insert(local.into());
                        self.data.imports.push(ImportBinding {
                            local_name: local.into(),
                            exported_name: "*".into(),
                            specifier: specifier.clone(),
                            is_type_only,
                        });
                    }
                }
            }
        }

        // Don't walk children — we've handled everything
    }

    /// `export { Foo }`: local specifiers, no source.
    fn visit_export_named_declaration(&mut self, node: &ExportNamedDeclaration<'a>) {
        for spec in &node.specifiers {
            // Recorded as a local declaration for now; `finish()` turns it into a re-export when `local` is an import.
            self.pending_local_exports.push((self.data.exports.len(), spec.local.name().as_str().to_owned()));
            self.data.exports.push(LexedExport::LocalDeclaration {
                name: spec.exported.name().as_str().to_owned(),
                is_type_only: node.export_kind.is_type() || spec.export_kind.is_type(),
            });
        }

        walk::walk_export_named_declaration(self, node);
    }

    /// `export { X } from "./y"`.
    fn visit_export_from_declaration(&mut self, node: &ExportFromDeclaration<'a>) {
        let src = node.source.value.as_str().to_owned();
        for spec in &node.specifiers {
            self.data.exports.push(LexedExport::ReExportNamed {
                // local_name is what we call it here; source_name is the original
                local_name: spec.exported.name().as_str().to_owned(),
                source_name: spec.local.name().as_str().to_owned(),
                source_specifier: src.clone(),
                is_type_only: node.export_kind.is_type() || spec.export_kind.is_type(),
            });
        }

        walk::walk_export_from_declaration(self, node);
    }

    /// `export interface Foo` / `export const X`. `is_type_only` is always false: `export_kind` only marks
    /// `export type { A }`, and `export enum` declares a runtime value.
    fn visit_export_declaration(&mut self, node: &ExportDeclaration<'a>) {
        if let Some(name) = declaration_name(&node.declaration) {
            self.data.exports.push(LexedExport::LocalDeclaration { name: name.to_owned(), is_type_only: false });
        }

        // Walk the declaration so sub-visitors (interface, type alias, etc.) fire
        walk::walk_export_declaration(self, node);
    }

    fn visit_export_all_declaration(&mut self, node: &ExportAllDeclaration<'a>) {
        let src = node.source.value.as_str().to_owned();
        if let Some(ns) = &node.exported {
            // `export * as Ns from "./y"`
            self.data.exports.push(LexedExport::ReExportNamespace {
                namespace: ns.name().as_str().to_owned(),
                source_specifier: src,
            });
        } else {
            // `export * from "./y"`
            self.data
                .exports
                .push(LexedExport::ReExportAll { source_specifier: src, is_type_only: node.export_kind.is_type() });
        }
    }

    fn visit_ts_interface_declaration(&mut self, node: &TSInterfaceDeclaration<'a>) {
        let name = node.id.name.as_str();
        let key = self.scoped_key(name);

        let extends: Vec<ExtendsRef> = node.extends.iter().map(|ext| self.collect_extends(ext)).collect();

        // Claim the interface's own leading comment before descending into props —
        // otherwise a short interface's first prop (processed next) can steal it via
        // find_jsdoc's proximity match, leaving the interface's own description empty.
        let (description, tags) = self.find_jsdoc_with_tags(node.span.start);

        let props: Vec<RawProp> =
            node.body.body.iter().filter_map(|sig| self.collect_property_signature(sig)).collect();

        self.data.interfaces.insert(
            key.clone(),
            CollectedInterface {
                scoped_key: key.clone(),
                name: name.into(),
                file_path: self.file_path.clone(),
                props,
                extends,
                description,
                tags,
            },
        );

        // Record declared type parameter names (`interface Foo<TData, TValue>` →
        // ["TData", "TValue"]) so the resolver can recognize bare references to
        // them inside the interface's own body as expected generic placeholders
        // rather than unresolvable types — see `resolver/chain.rs`.
        if let Some(type_parameters) = &node.type_parameters {
            let params: Vec<TypeName> = type_parameters.params.iter().map(|p| p.name.name.as_str().into()).collect();
            if !params.is_empty() {
                self.data.interface_type_params.insert(key, params);
            }
        }

        // Don't walk children — we've extracted everything we need
    }

    fn visit_ts_type_alias_declaration(&mut self, node: &TSTypeAliasDeclaration<'a>) {
        let name = node.id.name.as_str();
        let key = self.scoped_key(name);

        let Some(alias) = self.classify_type_alias(name, &node.type_annotation) else {
            return;
        };
        self.data.type_aliases.insert(key.clone(), alias);

        // Record declared type parameter names (`type Assign<T, U> = ...` → ["T", "U"])
        // so the resolver can substitute call-site arguments into the alias body —
        // see `resolver/substitute.rs`. Absent entry = non-generic alias.
        if let Some(type_parameters) = &node.type_parameters {
            let params: Vec<TypeName> = type_parameters.params.iter().map(|p| p.name.name.as_str().into()).collect();
            if !params.is_empty() {
                self.data.type_alias_params.insert(key, params);
            }
        }
    }

    /// `namespace Foo {}` qualifies inner names as `Foo.Bar`. `declare module "x"` and `declare global` are separate
    /// nodes and qualify nothing.
    fn visit_ts_namespace_declaration(&mut self, node: &TSNamespaceDeclaration<'a>) {
        self.namespace_stack.push(node.id.name.as_str().into());
        walk::walk_ts_namespace_declaration(self, node);
        self.namespace_stack.pop();
    }

    fn visit_ts_enum_declaration(&mut self, node: &TSEnumDeclaration<'a>) {
        let enum_name = node.id.name.as_str();
        let key = self.scoped_key(enum_name);

        let entries: Vec<EnumEntry> = node
            .body
            .members
            .iter()
            .filter_map(|member| {
                let name = match &member.id {
                    TSEnumMemberName::Identifier(id) => id.name.as_str().to_owned(),
                    TSEnumMemberName::String(s) => s.value.as_str().to_owned(),
                    TSEnumMemberName::ComputedString(s) => s.value.as_str().to_owned(),
                    TSEnumMemberName::ComputedTemplateString(_) => return None,
                };

                let value = match &member.initializer {
                    Some(init) => {
                        self.expression_to_enum_value(init).unwrap_or_else(|| EnumValue::String(name.clone()))
                    }
                    None => EnumValue::String(name.clone()),
                };

                let description = self.find_jsdoc(member.span.start);
                Some(EnumEntry { name, value, description })
            })
            .collect();

        if !entries.is_empty() {
            self.data.enums.insert(key, entries);
        }
    }

    fn visit_variable_declaration(&mut self, node: &VariableDeclaration<'a>) {
        for declarator in &node.declarations {
            self.try_collect_const_enum(declarator);
            self.try_collect_const_array(declarator);
            // Collect cva() / tv() variant definitions for all file types (.ts and .tsx)
            if let BindingPattern::BindingIdentifier(id) = &declarator.id {
                let name = id.name.as_str().to_owned();
                self.try_collect_cva_call(declarator, &name);
            }
            if self.is_tsx {
                if let Some(name) = self.extract_pascal_name(declarator) {
                    self.decline_reason = None;
                    let binding = Binding::of(declarator);
                    if let Some(mapping) = self.detect_component(&binding, &name) {
                        self.data.component_mappings.push(mapping);
                        continue;
                    }
                    // try_rename_identifier_wrapped_component is itself a give-up-quietly
                    // path (a bare/wrapped identifier re-binding, not a props-bearing
                    // component candidate) — only record a skip when even that finds
                    // nothing, so plain aliasing (`const Button = InternalButton;`)
                    // doesn't spuriously report as an unsupported candidate. Also skip
                    // no-initializer declarations (`declare var Date: DateConstructor`) —
                    // those are ambient type-only bindings handled by Pattern 5 below
                    // (or legitimately not components at all), not failed candidates.
                    if declarator.init.is_some()
                        && !self.try_rename_identifier_wrapped_component(&binding, &name)
                        && !self.try_record_wrapper_call(&binding, &name)
                    {
                        let message = self.decline_reason.take().unwrap_or_else(|| {
                            format!(
                                "'{name}' is a PascalCase binding but matched no known component pattern \
                                 (FC annotation, forwardRef, HOC wrapper, or identifier alias)"
                            )
                        });
                        self.record_skip(DiagnosticCode::SkippedCandidate, message, declarator.span);
                    }
                }
            }
            // Pattern 5: declare const Button: React.ForwardRefExoticComponent<ButtonProps & RefAttributes<E>>
            // Common in .d.ts files — no initializer, just a type annotation
            if declarator.init.is_none() {
                if let Some(name) = self.extract_pascal_name(declarator) {
                    if let Some(mapping) = self.try_forward_ref_exotic_decl(declarator, &name) {
                        self.data.component_mappings.push(mapping);
                    } else if self.type_annotation_is_fc_family(declarator) {
                        // try_fc_annotation already ran unconditionally above and found
                        // nothing — the annotation is FC-shaped but its props type
                        // argument is an exotic TSType this extractor doesn't match.
                        self.record_skip(
                            DiagnosticCode::SkippedCandidate,
                            format!(
                                "'{name}' is a PascalCase declaration with an FC-family type annotation whose \
                                 props type argument isn't a recognizable type"
                            ),
                            declarator.span,
                        );
                    }
                }
            }
        }
        walk::walk_variable_declaration(self, node);
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        // Pattern 4: `function Button(props: ButtonProps) { ... }`
        if self.is_tsx {
            if let Some(id) = &func.id {
                let name = id.name.as_str();
                let declaration =
                    matches!(func.r#type, FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction);
                if is_pascal_case(name) && declaration {
                    if let Some(first_param) = func.params.items.first() {
                        if let Some(type_ann) = &first_param.type_annotation {
                            if let Err(reason) =
                                super::evidence::renders(&super::evidence::FunctionShape::of_function(func), &|n| {
                                    self.may_be_react(n)
                                })
                            {
                                // A `declare function` with a non-element return type is an API declaration, not a
                                // component that failed to qualify: lib.dom.d.ts alone declares dozens of them.
                                if func.r#type == FunctionType::FunctionDeclaration {
                                    self.record_skip(
                                        DiagnosticCode::SkippedCandidate,
                                        format!("'{name}' takes a typed props parameter but {reason}"),
                                        func.span,
                                    );
                                }
                            } else if let Some((props_name, type_args)) =
                                self.extract_type_name_from_type(&type_ann.type_annotation)
                            {
                                if func.params.items.len() > 1 {
                                    self.takes_a_second_parameter.insert(name.into());
                                }
                                let (description, tags) = self.find_jsdoc_with_tags(func.span.start);
                                let param_defaults = self.extract_param_defaults(&func.params);
                                self.data.component_mappings.push(
                                    ComponentMapping::new(
                                        name.to_owned(),
                                        props_name,
                                        self.file_path.clone(),
                                        func.span.start,
                                        func.span.end,
                                        DetectionRule::FunctionDeclaration,
                                    )
                                    .with_props_type_args(type_args)
                                    .with_doc(description, tags)
                                    .with_param_defaults(param_defaults),
                                );
                            } else {
                                self.record_skip(
                                    DiagnosticCode::SkippedCandidate,
                                    format!(
                                        "'{name}' is a PascalCase function declaration whose first param's type \
                                         annotation isn't a recognizable props type reference"
                                    ),
                                    type_ann.span,
                                );
                            }
                        } else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{name}' is a PascalCase function declaration with an untyped first param"),
                                first_param.span,
                            );
                        }
                    }
                }
            }
        }
        walk::walk_function(self, func, flags);
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        // `class Button extends React.Component<Props> { ... }` — shared entry
        // point for both a ClassDeclaration and a self-named ClassExpression;
        // an anonymous ClassExpression (no `class.id`) is instead picked up by
        // `try_class_expression_wrapped` in `visit_variable_declaration`'s
        // detector chain, from the outer binding's name.
        if self.is_tsx {
            if let Some(id) = &class.id {
                let name = id.name.as_str();
                if is_pascal_case(name) {
                    if let Some(family) = self.super_class_is_component_family(class) {
                        if let Some(mapping) = self.try_class_component(class, name) {
                            self.data.component_mappings.push(mapping);
                        } else if class.heritage.as_ref().is_some_and(|h| h.type_arguments.is_some()) {
                            // Extends Component/PureComponent WITH type args, but the
                            // props argument itself is an exotic shape
                            // extract_type_name_from_type doesn't match. No type args
                            // at all is left silent — mirrors visit_function's Pattern 4
                            // zero-params contract (a genuinely untyped candidate, not a
                            // malformed one).
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!(
                                    "'{name}' is a class component extending {family} whose props type \
                                     argument isn't a recognizable props type reference"
                                ),
                                class.span,
                            );
                        }
                    }
                }
            }
        }
        walk::walk_class(self, class);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.record_reference(it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        self.record_call(it);
        walk::walk_call_expression(self, it);
    }

    fn visit_jsx_opening_element(&mut self, it: &JSXOpeningElement<'a>) {
        self.record_jsx_element(it);
        walk::walk_jsx_opening_element(self, it);
    }

    fn visit_export_default_declaration(&mut self, node: &ExportDefaultDeclaration<'a>) {
        match &node.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                if let Some(id) = &func.id {
                    self.record_exported_declaration(id.name.as_str());
                }
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    self.record_exported_declaration(id.name.as_str());
                }
            }
            _ => {}
        }
        // An anonymous default export has no binding name, so the component is named after its file, as
        // `try_anonymous_default_export_component` already does for `export default function(props: Props) {}`.
        if self.is_tsx {
            match &node.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(func) if func.id.is_none() => {
                    self.try_anonymous_default_export_component(func);
                }
                ExportDefaultDeclarationKind::ClassDeclaration(class) if class.id.is_none() => {
                    if let Some(name) = super::component::pascal_case_from_file_stem(&self.file_path) {
                        if let Some(mapping) = self.try_class_component(class, &name) {
                            self.data.component_mappings.push(mapping);
                        }
                    }
                }
                // `export default (props: P) => …`, `export default memo(function Badge(…))`,
                // `export default forwardRef<El, P>(…)`: an expression, so the declarator rules apply.
                kind => {
                    if let (Some(expr), Some(name)) =
                        (kind.as_expression(), super::component::pascal_case_from_file_stem(&self.file_path))
                    {
                        let binding = Binding { init: Some(expr), span: node.span, annotation: None };
                        self.decline_reason = None;
                        if let Some(mapping) = self.detect_component(&binding, &name) {
                            self.data.component_mappings.push(mapping);
                        } else {
                            self.try_record_wrapper_call(&binding, &name);
                        }
                    }
                }
            }
        }
        walk::walk_export_default_declaration(self, node);
    }

    fn visit_expression_statement(&mut self, node: &ExpressionStatement<'a>) {
        self.try_scan_display_name(node);
        self.try_scan_default_props(node);
        if self.is_tsx {
            self.try_scan_object_assign_sub_components(node);
        }
        walk::walk_expression_statement(self, node);
    }
}
