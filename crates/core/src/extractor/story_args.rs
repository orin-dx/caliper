//! The `args` a stories file sets, read from the syntax alone (ADR 0028, section A, slice T6): the keys of the default
//! export's `args` and of every story's, with what the default export's `component` names. Names only, as written;
//! which component that is, is resolved by symbol and import with the rest of the call-site evidence
//! (`relevance::call_sites`).
//!
//! The forms are those of Component Story Format 3, typed or not: a default export that is an object literal or the
//! name of a module-level `const` holding one, also through `export { meta as default }` (`const meta: Meta<typeof Button> = {…}`, `{…} satisfies Meta`,
//! `{…} as Meta`); stories as exported `const`s holding object literals (`export const X: StoryObj = { args: {…} }`,
//! or exported by `export { X }`); `X.args = {…}` statements at the top level for an exported story `X`; and the CSF
//! factories' object arguments (`preview.meta({…})`, `meta.story({…})`). Only an `args` object literal's static keys
//! count; a spread in it (`...Other.args`) passes names the syntax does not follow.
#![cfg_attr(not(test), expect(dead_code, reason = "wired into the pipeline by ADR 0028 slice T7"))]

use camino::Utf8Path;
use oxc_ast::ast::*;
use rustc_hash::{FxHashMap, FxHashSet};

use super::component::peel_expr;
use super::js_text;

/// The `args` of one stories file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StoryArgs {
    /// What the default export's `component` names, as written (`Button`, `Menu.Item`); `None` when it names nothing
    /// a tag could (no `component`, or another expression).
    pub(crate) component: Option<String>,
    /// The keys of the default export's `args`, then of each story's, in source order, each once.
    pub(crate) args: Vec<String>,
}

/// Whether `path` is a stories file: its name has a `.stories.` part (`button.stories.tsx`).
pub(crate) fn is_stories_file(path: &Utf8Path) -> bool {
    path.file_name().is_some_and(|name| name.contains(".stories."))
}

/// Reads a stories file's `args`; `None` when it has no default export that is an object literal.
pub(crate) fn read(program: &Program<'_>) -> Option<StoryArgs> {
    let mut consts: FxHashMap<&str, &ObjectExpression<'_>> = FxHashMap::default();
    // The bindings exported as stories, and the one exported as the meta through `export { meta as default }`.
    let (mut stories, mut meta_binding) = (FxHashSet::default(), None);
    for statement in &program.body {
        let (declaration, exported) = match statement {
            Statement::VariableDeclaration(declaration) => (declaration, false),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::VariableDeclaration(declaration) => (declaration, true),
                _ => continue,
            },
            Statement::ExportNamedDeclaration(export) => {
                for specifier in &export.specifiers {
                    let local = specifier.local.name().as_str();
                    if specifier.exported.name() == "default" {
                        meta_binding = Some(local);
                    } else {
                        stories.insert(local);
                    }
                }
                continue;
            }
            _ => continue,
        };
        if declaration.kind != VariableDeclarationKind::Const {
            continue;
        }
        for declarator in &declaration.declarations {
            let BindingPattern::BindingIdentifier(id) = &declarator.id else { continue };
            if exported {
                stories.insert(id.name.as_str());
            }
            if let Some(object) = declarator.init.as_ref().and_then(object) {
                consts.insert(id.name.as_str(), object);
            }
        }
    }
    let meta = program
        .body
        .iter()
        .find_map(|statement| match statement {
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::Identifier(id) => consts.get(id.name.as_str()).copied(),
                declaration => object(declaration.as_expression()?),
            },
            _ => None,
        })
        .or_else(|| consts.get(meta_binding?).copied())?;
    let mut read = StoryArgs { component: property(meta, "component").and_then(path), args: Vec::new() };
    read.add(property(meta, "args"));
    for statement in &program.body {
        let names: Vec<&str> = match statement {
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::VariableDeclaration(declaration) => declaration
                    .declarations
                    .iter()
                    .filter_map(|declarator| match &declarator.id {
                        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
                        _ => None,
                    })
                    .collect(),
                _ => continue,
            },
            Statement::ExportNamedDeclaration(export) => export
                .specifiers
                .iter()
                .filter(|specifier| specifier.exported.name() != "default")
                .map(|specifier| specifier.local.name().as_str())
                .collect(),
            Statement::ExpressionStatement(statement) => {
                let Expression::AssignmentExpression(assignment) = &statement.expression else { continue };
                let AssignmentTarget::StaticMemberExpression(target) = &assignment.left else { continue };
                if target.property.name == "args"
                    && let Expression::Identifier(story) = &target.object
                    && (stories.contains(story.name.as_str()) || meta_binding == Some(story.name.as_str()))
                {
                    read.add(Some(&assignment.right));
                }
                continue;
            }
            _ => continue,
        };
        for name in names {
            read.add(consts.get(name).and_then(|story| property(story, "args")));
        }
    }
    Some(read)
}

impl StoryArgs {
    /// Adds the static keys of `args` when it is an object literal.
    fn add(&mut self, args: Option<&Expression<'_>>) {
        let Some(args) = args.and_then(object) else { return };
        for property in &args.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else { continue };
            let Some(key) = property.key.static_name().map(|key| js_text::property_name(&key)) else { continue };
            if !self.args.contains(&key) {
                self.args.push(key);
            }
        }
    }
}

/// The object literal `expression` is, through parentheses, `as`, `satisfies` and `!`, or the one a CSF factory call
/// takes (`preview.meta({…})`, `meta.story({…})`).
fn object<'a, 'b>(expression: &'b Expression<'a>) -> Option<&'b ObjectExpression<'a>> {
    match peel_expr(expression) {
        Expression::ObjectExpression(object) => Some(object),
        Expression::CallExpression(call) if call.arguments.len() == 1 && is_factory(&call.callee) => {
            object(call.arguments[0].as_expression()?)
        }
        _ => None,
    }
}

/// Whether `callee` is a CSF factory: a `meta` or `story` method.
fn is_factory(callee: &Expression<'_>) -> bool {
    matches!(callee, Expression::StaticMemberExpression(member) if matches!(member.property.name.as_str(), "meta" | "story"))
}

/// The value of the last property with the static key `key`.
fn property<'a, 'b>(object: &'b ObjectExpression<'a>, key: &str) -> Option<&'b Expression<'a>> {
    object.properties.iter().rev().find_map(|property| match property {
        ObjectPropertyKind::ObjectProperty(property) if property.key.static_name().is_some_and(|name| name == key) => {
            Some(&property.value)
        }
        _ => None,
    })
}

/// The dotted path an identifier or a chain of static member reads spells (`Menu.Item`).
fn path(expression: &Expression<'_>) -> Option<String> {
    match peel_expr(expression) {
        Expression::Identifier(id) => Some(id.name.to_string()),
        Expression::StaticMemberExpression(member) => {
            Some(format!("{}.{}", path(&member.object)?, member.property.name))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::*;

    fn args_of(source: &str) -> Option<StoryArgs> {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, source, SourceType::tsx()).parse();
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        read(&parsed.program)
    }

    fn expect(source: &str, component: &str, args: &[&str]) {
        let read = args_of(source).expect("a default export");
        assert_eq!(read.component.as_deref(), Some(component), "{source}");
        assert_eq!(read.args, args, "{source}");
    }

    #[test]
    fn meta_args_and_story_object_args_are_read() {
        expect(
            "import { Button } from './button';
             export default { component: Button, args: { tone: 'a' } };
             export const Primary = { args: { size: 'b', tone: 'c' } };
             export const Plain = {};",
            "Button",
            &["tone", "size"],
        );
    }

    #[test]
    fn typed_meta_through_a_const_satisfies_or_as() {
        expect(
            "const meta: Meta<typeof Button> = { component: Button, args: { a: 1 } };
             export default meta;
             type Story = StoryObj<typeof meta>;
             export const X: Story = { args: { b: 2 } };",
            "Button",
            &["a", "b"],
        );
        expect(
            "const meta = { component: Button } satisfies Meta<typeof Button>;
             export default meta;
             export const X = { args: { 'c-d': 1, ['e']: 2 } } satisfies Story;",
            "Button",
            &["c-d", "e"],
        );
        expect("export default ({ component: Menu.Item, args: { f: 1 } } as Meta);", "Menu.Item", &["f"]);
    }

    #[test]
    fn assigned_args_and_re_exported_stories_are_read() {
        expect(
            "export default { component: Button };
             const Template = (args) => <Button {...args} />;
             export const A = Template.bind({});
             A.args = { g: 1, ...Other.args };
             const B = { args: { h: 2 } };
             export { B };",
            "Button",
            &["g", "h"],
        );
    }

    #[test]
    fn a_meta_exported_by_a_specifier_and_csf_factories_are_read() {
        expect(
            "const meta = { component: Button, args: { a: 1 } };
             const X = { args: { b: 2 } };
             export { meta as default, X };",
            "Button",
            &["a", "b"],
        );
        expect(
            "const meta = preview.meta({ component: Button, args: { a: 1 } });
             export default meta;
             export const X = meta.story({ args: { b: 2 } });",
            "Button",
            &["a", "b"],
        );
    }

    #[test]
    fn args_assigned_to_a_binding_that_is_not_an_exported_story_are_not_read() {
        expect(
            "export default { component: Button };
             const Template = (args) => <Button {...args} />;
             Template.args = { debugOnly: true };
             export const A = Template.bind({});",
            "Button",
            &[],
        );
    }

    #[test]
    fn spreads_computed_keys_and_non_literal_args_are_not_read() {
        expect(
            "export default { component: Button, args: base };
             export const X = { args: { ...base, [key]: 1, i: 2 } };",
            "Button",
            &["i"],
        );
    }

    #[test]
    fn a_meta_without_a_component_or_a_file_without_a_meta() {
        assert_eq!(args_of("export default { args: { a: 1 } };").map(|read| read.component), Some(None));
        assert_eq!(args_of("export default { component: lazy(() => x) };").and_then(|read| read.component), None);
        assert_eq!(args_of("export const X = { args: { a: 1 } };"), None);
        assert_eq!(args_of("export default function Page() { return null; }"), None);
    }

    #[test]
    fn stories_files_are_named_so() {
        assert!(is_stories_file("src/button.stories.tsx".into()));
        assert!(is_stories_file("src/Button.stories.mdx.tsx".into()));
        assert!(!is_stories_file("src/stories/button.tsx".into()));
    }
}
