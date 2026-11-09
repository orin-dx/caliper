//! `impl Visit for SourceDataCollector` — the AST walker entry points.

use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_syntax::scope::ScopeFlags;

use crate::types::{
    CollectedInterface, ComponentMapping, DetectionRule, DiagnosticCode, EnumEntry, EnumValue, ExtendsRef,
    ImportBinding, LexedExport, RawProp, ScopeKind, TypeName,
};

use super::component::Binding;
use super::{SourceDataCollector, declaration_name, is_constant_name, is_pascal_case, js_text};

impl<'a, 'src> Visit<'a> for SourceDataCollector<'src> {
    fn visit_import_declaration(&mut self, node: &ImportDeclaration<'a>) {
        let specifier = js_text::lossy(node.source.value).into_owned();
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
            self.record_exported_local(spec.local.name().as_str());
            // Recorded as a local declaration for now; `finish()` turns it into a re-export when `local` is an import.
            let (local, exported) = (spec.local.name().as_str(), spec.exported.name().as_str());
            self.pending_local_exports.push((self.data.exports.len(), local.to_owned()));
            self.data.exports.push(LexedExport::LocalDeclaration {
                name: exported.to_owned(),
                is_type_only: node.export_kind.is_type() || spec.export_kind.is_type(),
                declared_as: (local != exported).then(|| local.to_owned()),
            });
            if let Some((base_name, props_name)) = self.cast_component_aliases.get(local).cloned()
                && let Some(index) =
                    self.data.component_mappings.iter().position(|mapping| mapping.component_name == base_name)
            {
                let mut public = self.data.component_mappings[index].clone();
                public.component_name = exported.to_owned();
                public.props_type_name = props_name;
                public.props_type_args.clear();
                public.wrapper_callee = None;
                public.rule = crate::types::DetectionRule::FcAnnotation;
                // `const A = Inner as VFC<P>` was already bound as an alias of `Inner`; the cast replaces that
                // mapping so `A` is published once, with `P`.
                // A rename (`export { A as B }`) replaces only the mapping of the inner component being re-exported
                // under its own name; another local that happens to be called `B` keeps its mapping.
                let existing = (local == exported || base_name == exported)
                    .then(|| self.data.component_mappings.iter().position(|mapping| mapping.component_name == exported))
                    .flatten();
                if let Some(at) = existing {
                    self.data.component_mappings[at] = public;
                } else {
                    self.data.component_mappings.push(public);
                }
                self.record_exported_local(exported);
            }
        }

        walk::walk_export_named_declaration(self, node);
    }

    /// `export { X } from "./y"`.
    fn visit_export_from_declaration(&mut self, node: &ExportFromDeclaration<'a>) {
        let src = js_text::lossy(node.source.value).into_owned();
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
            self.data.exports.push(LexedExport::LocalDeclaration {
                name: name.to_owned(),
                is_type_only: false,
                declared_as: None,
            });
        }
        // `export const A = …, B = …` exports every declarator.
        match &node.declaration {
            Declaration::VariableDeclaration(vd) => {
                for declarator in &vd.declarations {
                    match &declarator.id {
                        BindingPattern::BindingIdentifier(id) => self.record_exported_local(id.name.as_str()),
                        // `export const { Option, Item: Row } = Imported` exports `Option` and `Row`.
                        BindingPattern::ObjectPattern(pattern) => {
                            for property in &pattern.properties {
                                let BindingPattern::BindingIdentifier(local) = &property.value else { continue };
                                self.data.exports.push(LexedExport::LocalDeclaration {
                                    name: local.name.as_str().to_owned(),
                                    is_type_only: false,
                                    declared_as: None,
                                });
                                self.record_exported_local(local.name.as_str());
                            }
                        }
                        _ => {}
                    }
                }
            }
            declaration => {
                if let Some(name) = declaration_name(declaration) {
                    self.record_exported_local(name);
                }
            }
        }

        // Walk the declaration so sub-visitors (interface, type alias, etc.) fire
        walk::walk_export_declaration(self, node);
    }

    fn visit_export_all_declaration(&mut self, node: &ExportAllDeclaration<'a>) {
        let src = js_text::lossy(node.source.value).into_owned();
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
        let scope = self.current_scope();
        let mut key = self.scoped_key(name);
        key.extend(scope.local_key_suffix());

        let extends: Vec<ExtendsRef> = node.extends.iter().map(|ext| self.collect_extends(ext)).collect();

        // Claim the interface's own leading comment before descending into props —
        // otherwise a short interface's first prop (processed next) can steal it via
        // find_jsdoc's proximity match, leaving the interface's own description empty.
        let (description, tags) = self.find_jsdoc_with_tags(node.span.start);

        let accessors = super::Accessors::of(&node.body.body);
        let props: Vec<RawProp> = node
            .body
            .body
            .iter()
            .filter(|sig| !accessors.is_shadowed_setter(sig))
            .filter_map(|sig| self.collect_property_signature(sig))
            .collect();

        // Declarations of one name in one scope merge, as TypeScript merges them: members and `extends` accumulate,
        // and the first description stands. The key carries the scope, so a declaration in another scope is an entry
        // of its own.
        if let Some(merged) = self.data.interfaces.get_mut(&key) {
            for prop in props {
                if !merged.props.iter().any(|existing| existing.name == prop.name) {
                    merged.props.push(prop);
                }
            }
            merged.extends.extend(extends);
            if merged.description.is_empty() {
                merged.description = description;
            }
            for (tag, value) in tags {
                merged.tags.entry(tag).or_insert(value);
            }
        } else {
            self.data.interfaces.insert(
                key.clone(),
                CollectedInterface {
                    scoped_key: key.clone(),
                    name: name.into(),
                    scope,
                    file_path: self.file_path.clone(),
                    props,
                    extends,
                    description,
                    tags,
                },
            );
        }

        // Record declared type parameter names (`interface Foo<TData, TValue>` →
        // ["TData", "TValue"]) so the resolver can recognize bare references to
        // them inside the interface's own body as expected generic placeholders
        // rather than unresolvable types — see `resolver/chain.rs`.
        if let Some(type_parameters) = &node.type_parameters {
            let params: Vec<TypeName> = type_parameters.params.iter().map(|p| p.name.name.as_str().into()).collect();
            if !params.is_empty() {
                self.record_type_param_defaults(&key, &type_parameters.params);
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
        let scope = self.current_scope();
        self.data.type_alias_scopes.insert(key.clone(), scope);

        // Record declared type parameter names (`type Assign<T, U> = ...` → ["T", "U"])
        // so the resolver can substitute call-site arguments into the alias body —
        // see `resolver/substitute.rs`. Absent entry = non-generic alias.
        if let Some(type_parameters) = &node.type_parameters {
            let params: Vec<TypeName> = type_parameters.params.iter().map(|p| p.name.name.as_str().into()).collect();
            if !params.is_empty() {
                self.record_type_param_defaults(&key, &type_parameters.params);
                self.data.type_alias_params.insert(key, params);
            }
        }
    }

    /// `namespace Foo {}` qualifies inner names as `Foo.Bar`. `declare module "x"` and `declare global` are separate
    /// nodes and qualify nothing.
    fn visit_ts_namespace_declaration(&mut self, node: &TSNamespaceDeclaration<'a>) {
        self.namespace_stack.push(node.id.name.as_str().into());
        self.ambient_depth += u32::from(node.declare);
        self.enter_scope(ScopeKind::Namespace);
        walk::walk_ts_namespace_declaration(self, node);
        self.leave_scope();
        self.ambient_depth -= u32::from(node.declare);
        self.qualify_namespace_references();
        self.namespace_stack.pop();
    }

    /// `declare module "react" { interface HTMLAttributes<T> { ... } }` merges into React's `React` namespace, so its
    /// declarations are keyed as `React.`-qualified ones (`NamedTypeIndex::react_augmentations` reads them). Any other
    /// `declare module "x"` qualifies nothing.
    fn visit_ts_external_module_declaration(&mut self, node: &TSExternalModuleDeclaration<'a>) {
        self.ambient_depth += 1;
        self.enter_scope(ScopeKind::Namespace);
        if node.id.value != "react" {
            walk::walk_ts_external_module_declaration(self, node);
            self.leave_scope();
            self.ambient_depth -= 1;
            return;
        }
        self.namespace_stack.push("React".into());
        walk::walk_ts_external_module_declaration(self, node);
        self.leave_scope();
        self.ambient_depth -= 1;
        self.qualify_namespace_references();
        self.namespace_stack.pop();
    }

    fn visit_ts_enum_declaration(&mut self, node: &TSEnumDeclaration<'a>) {
        // An enum declared in a function body or a block is not the enum a module's types name: `scoped_key` ignores
        // those scopes, so its union and entries would merge into, or replace, a module-level enum of the same name.
        if self.function_depth > 0 || self.block_depth > 0 {
            return;
        }
        let enum_name = node.id.name.as_str();
        let key = self.scoped_key(enum_name);

        let values = self.enum_member_values(node);
        let entries: Vec<EnumEntry> = node
            .body
            .members
            .iter()
            .zip(&values)
            .filter_map(|(member, value)| {
                let name = match &member.id {
                    TSEnumMemberName::Identifier(id) => id.name.as_str().to_owned(),
                    TSEnumMemberName::String(s) => js_text::lossy(s.value).into_owned(),
                    TSEnumMemberName::ComputedString(s) => js_text::lossy(s.value).into_owned(),
                    TSEnumMemberName::ComputedTemplateString(_) => return None,
                };
                let value = value.clone().unwrap_or_else(|| EnumValue::String(name.clone()));
                let description = self.find_jsdoc(member.span.start);
                Some(EnumEntry { name, value, description })
            })
            .collect();

        self.record_enum_union(&key, &values);
        if !entries.is_empty() {
            let scope = self.current_scope();
            self.data.enum_declarations.insert(key.clone(), scope);
            self.data.enums.entry(key).or_default().extend(entries);
        }
    }

    fn visit_variable_declaration(&mut self, node: &VariableDeclaration<'a>) {
        for declarator in &node.declarations {
            self.try_collect_const_enum(declarator);
            self.try_collect_const_array(declarator);
            // Collect variant bags (a call with a `variants` map of maps: cva, tv, recipes) for all file types
            if let BindingPattern::BindingIdentifier(id) = &declarator.id {
                let name = id.name.as_str().to_owned();
                self.try_collect_variant_bag(declarator, &name);
                self.record_object_assign_declarator(&name, declarator.init.as_ref());
                if self.is_tsx && self.function_depth == 0 {
                    self.try_scan_object_assign_declarator(declarator.init.as_ref());
                }
                if self.function_depth == 0 {
                    self.try_record_cast_component_alias(declarator, &name);
                    if let Some(init) = declarator.init.as_ref() {
                        self.record_bound_function_result(&name, init);
                    }
                }
                if self.function_depth == 0
                    && node.kind.is_const()
                    && let Some(Expression::Identifier(target)) =
                        declarator.init.as_ref().map(super::component::peel_expr)
                {
                    self.pending_function_aliases.push((name, target.name.as_str().to_owned()));
                }
            }
            if self.is_tsx && self.function_depth == 0 {
                self.try_record_destructured_imports(declarator);
            }
            if self.is_tsx && self.function_depth > 0 {
                if let Some(name) = self.extract_pascal_name(declarator)
                    && self.detect_component(&Binding::of(declarator), &name).is_some()
                {
                    self.skip_local_component(&name, declarator.span);
                }
            } else if self.is_tsx
                && let Some(name) = self.extract_pascal_name(declarator)
            {
                self.decline_reason = None;
                self.argument_declined = false;
                self.alias_candidate = None;
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
                    && !self.try_record_tagged_factory(&binding, &name)
                    && !self.try_record_imported_member_alias(&binding, &name)
                    && !self.try_record_factory_call(&binding, &name)
                    && !self.try_defer_identifier_alias(&name, declarator.span)
                {
                    // `MENU_OPTIONS` is a constant, not a component that failed to qualify; a rule that gave a
                    // reason for declining it still reports it.
                    let declined = self.decline_reason.take();
                    if declined.is_some() || !is_constant_name(&name) {
                        let message = declined.unwrap_or_else(|| {
                            format!(
                                "'{name}' is a PascalCase binding but matched no known component pattern \
                                     (FC annotation, forwardRef, HOC wrapper, or identifier alias)"
                            )
                        });
                        self.record_skip(DiagnosticCode::SkippedCandidate, message, declarator.span);
                    }
                }
            }
            self.record_body_defaults(declarator);
            if node.kind.is_const() {
                self.record_const_value_type(declarator);
            }
            // Pattern 5: declare const Button: React.ForwardRefExoticComponent<ButtonProps & RefAttributes<E>>
            // Common in .d.ts files — no initializer, just a type annotation
            if declarator.init.is_none() {
                if node.kind.is_const() {
                    self.record_declared_value_type(declarator);
                }
                let name = self.extract_pascal_name(declarator).or_else(|| {
                    let path = self.file_path.as_str();
                    if ![".d.ts", ".d.mts", ".d.cts"].iter().any(|suffix| path.ends_with(suffix)) {
                        return None;
                    }
                    match &declarator.id {
                        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str().to_owned()),
                        _ => None,
                    }
                });
                if let Some(name) = name {
                    if let Some(mapping) = self
                        .try_exotic_component_decl(declarator, &name)
                        .or_else(|| self.try_declared_callable_component(declarator, &name))
                    {
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

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.record_function_type_params(arrow.span, arrow.type_parameters.as_deref());
        self.props_params.push((first_param_name(&arrow.params), arrow.span.start, arrow.span.end));
        walk::walk_arrow_function_expression(self, arrow);
        self.props_params.pop();
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        self.record_function_result(func);
        self.record_function_type_params(func.span, func.type_parameters.as_deref());
        self.props_params.push((first_param_name(&func.params), func.span.start, func.span.end));
        // Pattern 4: `function Button(props: ButtonProps) { ... }`
        if self.is_tsx
            && let Some(id) = &func.id
        {
            let name = id.name.as_str();
            let declaration =
                matches!(func.r#type, FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction);
            if is_pascal_case(name) && declaration && self.function_depth > 0 {
                let typed = func.params.items.first().is_some_and(|p| p.type_annotation.is_some());
                let shape = super::evidence::FunctionShape::of_function(func);
                if typed && super::evidence::renders(&shape, &*self).is_ok() {
                    self.skip_local_component(name, func.span);
                }
            } else if is_pascal_case(name)
                && declaration
                && let Some(first_param) = func.params.items.first()
            {
                if let Some(type_ann) = &first_param.type_annotation {
                    if let Err(reason) =
                        super::evidence::renders(&super::evidence::FunctionShape::of_function(func), &*self)
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
        walk::walk_function(self, func, flags);
        self.props_params.pop();
    }

    fn visit_block_statement(&mut self, block: &BlockStatement<'a>) {
        self.block_depth += 1;
        self.enter_scope(ScopeKind::Block);
        walk::walk_block_statement(self, block);
        self.leave_scope();
        self.block_depth -= 1;
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        self.block_depth += 1;
        self.enter_scope(ScopeKind::Block);
        walk::walk_static_block(self, block);
        self.leave_scope();
        self.block_depth -= 1;
    }

    fn visit_switch_statement(&mut self, switch: &SwitchStatement<'a>) {
        self.block_depth += 1;
        walk::walk_switch_statement(self, switch);
        self.block_depth -= 1;
    }

    /// Each `case` or `default` clause is a scope of its own for the declarations directly in it, though in
    /// JavaScript the whole `switch` body shares one lexical scope.
    fn visit_switch_case(&mut self, case: &SwitchCase<'a>) {
        self.enter_scope(ScopeKind::SwitchCase);
        walk::walk_switch_case(self, case);
        self.leave_scope();
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.function_depth += 1;
        self.enter_scope(ScopeKind::FunctionBody);
        if self.body_result_keys.iter().any(|(_, start)| *start == body.span.start) {
            let scope = self.current_scope();
            self.body_scopes.insert(body.span.start, scope);
        }
        walk::walk_function_body(self, body);
        self.leave_scope();
        self.function_depth -= 1;
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        // `class Button extends React.Component<Props> { ... }` — shared entry
        // point for both a ClassDeclaration and a self-named ClassExpression;
        // an anonymous ClassExpression (no `class.id`) is instead picked up by
        // `try_class_expression_wrapped` in `visit_variable_declaration`'s
        // detector chain, from the outer binding's name.
        if let Some(id) = &class.id {
            let params = class
                .type_parameters
                .as_ref()
                .map(|declaration| declaration.params.iter().map(|p| p.name.name.as_str().into()).collect())
                .unwrap_or_default();
            let scope = self.current_scope();
            self.pending_classes.push((self.scoped_key(id.name.as_str()), id.name.as_str().into(), params, scope));
        }
        if self.is_tsx && self.function_depth > 0 {
            if let Some(id) = &class.id {
                let name = id.name.as_str();
                if is_pascal_case(name) && self.super_class_is_component_family(class).is_some() {
                    self.skip_local_component(name, class.span);
                }
            }
        } else if self.is_tsx
            && let Some(id) = &class.id
        {
            let name = id.name.as_str();
            if is_pascal_case(name)
                && let Some(family) = self.super_class_is_component_family(class)
            {
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
        if let Some(expr) = node.declaration.as_expression() {
            self.record_asserted_props(expr);
        }
        // The default export's local name; an anonymous one is named after its file below.
        let local = match &node.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(func) => func.id.as_ref().map(|id| id.name.as_str()),
            ExportDefaultDeclarationKind::ClassDeclaration(class) => class.id.as_ref().map(|id| id.name.as_str()),
            kind => match kind.as_expression().map(super::component::peel_expr) {
                Some(Expression::Identifier(id)) => Some(id.name.as_str()),
                _ => None,
            },
        };
        match local {
            Some(name) => {
                self.record_exported_local(name);
                // `export default Tooltip`: the module's default is `Tooltip`, which `finish` turns into a re-export
                // when it is an import, as `export { Tooltip as default }` is.
                self.pending_local_exports.push((self.data.exports.len(), name.to_owned()));
                self.data.exports.push(LexedExport::LocalDeclaration {
                    name: "default".to_owned(),
                    is_type_only: false,
                    declared_as: Some(name.to_owned()),
                });
            }
            None => {
                if let Some(name) = super::component::pascal_case_from_file_stem(&self.file_path) {
                    self.record_exported_local(&name);
                }
            }
        }
        // An anonymous default export has no binding name, so the component is named after its file, as
        // `try_anonymous_default_export_component` already does for `export default function(props: Props) {}`.
        if self.is_tsx {
            match &node.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(func) if func.id.is_none() => {
                    self.try_anonymous_default_export_component(func);
                }
                ExportDefaultDeclarationKind::ClassDeclaration(class) if class.id.is_none() => {
                    if let Some(name) = super::component::pascal_case_from_file_stem(&self.file_path)
                        && let Some(mapping) = self.try_class_component(class, &name)
                    {
                        self.data.component_mappings.push(mapping);
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
                        self.argument_declined = false;
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
        self.record_member_assignment(node);
        if self.is_tsx {
            self.try_scan_object_assign_sub_components(node);
            self.try_scan_member_assigned_sub_component(node);
        }
        walk::walk_expression_statement(self, node);
    }
}

/// The name of a function's first parameter, when it is a plain identifier (`props`).
fn first_param_name(params: &FormalParameters<'_>) -> Option<compact_str::CompactString> {
    match &params.items.first()?.pattern {
        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str().into()),
        _ => None,
    }
}
