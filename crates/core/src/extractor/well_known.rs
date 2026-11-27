//! The well-known globals and imports the extractor recognises by name, as data. A name read plainly is matched as
//! origin/main matched it; a bare name read only through a wrapper origin/main did not read (`component::main_peel`) is
//! matched only when it resolves to the entry: a global the file never declares, or an import that is the name's one
//! declaration, from the entry's module. The architecture test below fails when an extractor source outside this module
//! spells one of these names as a string literal, so a new spelling-based path cannot skip the check.

use oxc_ast::ast::{Argument, AssignmentTarget, CallExpression, Expression, ObjectPropertyKind, PropertyKey};

use super::evidence::Scope;

/// How a well-known name resolves.
#[derive(Clone, Copy)]
pub(super) enum Kind {
    /// A global: the file must not declare the name at all. An import of the same name is another binding.
    Global,
    /// An import from `module`: the import must be the name's one declaration in the file. `namespace` entries are the
    /// module itself (`import * as React`, `import React`); the others are named exports, aliased or not (`import {
    /// forwardRef as fr }`).
    Import { module: Module, namespace: bool },
}

/// The modules an import entry may come from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Module {
    React,
    Preact,
}

impl Module {
    fn accepts(self, specifier: &str) -> bool {
        match self {
            Self::React => crate::react_types::is_react_module(specifier),
            Self::Preact => crate::react_types::is_preact_module(specifier),
        }
    }
}

/// A well-known name and how it resolves.
pub(super) struct WellKnown {
    pub(super) name: &'static str,
    pub(super) kind: Kind,
}

const fn global(name: &'static str) -> WellKnown {
    WellKnown { name, kind: Kind::Global }
}

const fn react(name: &'static str) -> WellKnown {
    WellKnown { name, kind: Kind::Import { module: Module::React, namespace: false } }
}

pub(super) const OBJECT: WellKnown = global("Object");
pub(super) const PROMISE: WellKnown = global("Promise");
pub(super) const REFLECT: WellKnown = global("Reflect");
pub(super) const ARRAY: WellKnown = global("Array");
pub(super) const REACT: WellKnown =
    WellKnown { name: "React", kind: Kind::Import { module: Module::React, namespace: true } };
pub(super) const PREACT: WellKnown =
    WellKnown { name: "Preact", kind: Kind::Import { module: Module::Preact, namespace: true } };
pub(super) const FORWARD_REF: WellKnown = react("forwardRef");
pub(super) const MEMO: WellKnown = react("memo");
pub(super) const LAZY: WellKnown = react("lazy");
pub(super) const CREATE_CONTEXT: WellKnown = react("createContext");
pub(super) const CREATE_ELEMENT: WellKnown = react("createElement");
pub(super) const CLONE_ELEMENT: WellKnown = react("cloneElement");
pub(super) const JSX: WellKnown = react("jsx");
pub(super) const JSXS: WellKnown = react("jsxs");
pub(super) const JSX_DEV: WellKnown = react("jsxDEV");
pub(super) const CREATE_PORTAL: WellKnown = react("createPortal");
pub(super) const H: WellKnown =
    WellKnown { name: "h", kind: Kind::Import { module: Module::Preact, namespace: false } };

/// Every entry: the one list the architecture test reads.
pub(super) const WELL_KNOWN: &[&WellKnown] = &[
    &OBJECT,
    &PROMISE,
    &REFLECT,
    &ARRAY,
    &REACT,
    &PREACT,
    &FORWARD_REF,
    &MEMO,
    &LAZY,
    &CREATE_CONTEXT,
    &CREATE_ELEMENT,
    &CLONE_ELEMENT,
    &JSX,
    &JSXS,
    &JSX_DEV,
    &CREATE_PORTAL,
    &H,
];

/// The callees that produce an element by name, read plainly as origin/main read them. Preact's `h` is not here: so
/// short a name proves nothing until it is known to be imported from Preact (`Scope::is_preact_h`).
pub(super) const ELEMENT_FACTORIES: &[&WellKnown] =
    &[&CREATE_ELEMENT, &CLONE_ELEMENT, &JSX, &JSXS, &JSX_DEV, &CREATE_PORTAL];

/// Whether `expr` is written as the entry's own name: origin/main's spelling match, for a name read plainly.
pub(super) fn is_named(expr: &Expression<'_>, entry: &WellKnown) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name == entry.name)
}

/// Whether `local`, a bare name read only through a wrapper, resolves to `entry` (see [`Kind`]).
pub(super) fn matches_well_known(scope: &dyn Scope, local: &str, entry: &WellKnown) -> bool {
    match entry.kind {
        Kind::Global => local == entry.name && scope.declarations(local) == 0,
        Kind::Import { module, namespace } => {
            scope.declarations(local) == 1
                && scope.import_of(local).is_some_and(|(exported, specifier)| {
                    module.accepts(specifier)
                        && if namespace { matches!(exported, "*" | "default") } else { exported == entry.name }
                })
        }
    }
}

/// Whether `local`, a bare callee read only through a wrapper, is the named import of one of `entries`.
pub(super) fn matches_any(scope: &dyn Scope, local: &str, entries: &[&WellKnown]) -> bool {
    entries.iter().any(|entry| matches_well_known(scope, local, entry))
}

/// The entry a React export name is, for a callee bound by an import whose exported name is `exported`.
pub(super) fn react_export(exported: &str) -> Option<&'static WellKnown> {
    WELL_KNOWN.iter().copied().find(|entry| {
        entry.name == exported && matches!(entry.kind, Kind::Import { module: Module::React, namespace: false })
    })
}

/// The imports a bare element factory callee may be: React's factories and Preact's `h`.
pub(super) const FACTORY_IMPORTS: &[&WellKnown] =
    &[&CREATE_ELEMENT, &CLONE_ELEMENT, &JSX, &JSXS, &JSX_DEV, &CREATE_PORTAL, &H];

/// The name and key a write to a member targets, for `names::Bindings::writes_to`: `Some(key)` for a key known
/// statically (`.assign`, `['assign']`, a template literal without substitutions), `None` for one that is not (`[k]`).
pub(super) fn assignment_write<'a>(target: &AssignmentTarget<'a>) -> Option<(&'a str, Option<&'a str>)> {
    match target {
        AssignmentTarget::StaticMemberExpression(member) => {
            object_name(&member.object).map(|object| (object, Some(member.property.name.as_str())))
        }
        AssignmentTarget::ComputedMemberExpression(member) => {
            object_name(&member.object).map(|object| (object, literal_key(&member.expression)))
        }
        _ => None,
    }
}

/// The writes a call makes to a member of a name: `Reflect.set(O, key, …)`, `Reflect.defineProperty(O, key, …)`,
/// `Object.defineProperty(O, key, …)` and each key of `Object.defineProperties(O, { … })`.
pub(super) fn call_writes<'a>(call: &CallExpression<'a>) -> Vec<(&'a str, Option<&'a str>)> {
    // The mutator and its target are read through every wrapper: a write behind one is still a write.
    let through = super::component::written_through;
    let Expression::StaticMemberExpression(callee) = through(&call.callee) else { return Vec::new() };
    let Some(receiver) = object_name(through(&callee.object)) else { return Vec::new() };
    let argument = |index: usize| call.arguments.get(index).and_then(Argument::as_expression);
    let Some(target) = argument(0).map(through).and_then(object_name) else { return Vec::new() };
    let global = receiver == OBJECT.name || receiver == REFLECT.name;
    match callee.property.name.as_str() {
        "set" | "defineProperty" if global => vec![(target, argument(1).and_then(literal_key))],
        // A new prototype may supply any member.
        "setPrototypeOf" if global => vec![(target, None)],
        "defineProperties" | "assign" if receiver == OBJECT.name => match argument(1).map(through) {
            Some(Expression::ObjectExpression(object)) => object
                .properties
                .iter()
                .map(|property| match property {
                    ObjectPropertyKind::ObjectProperty(property) if !property.computed => {
                        let key = match &property.key {
                            PropertyKey::StaticIdentifier(key) => Some(key.name.as_str()),
                            PropertyKey::StringLiteral(key) => key.value.as_str(),
                            _ => None,
                        };
                        (target, key)
                    }
                    _ => (target, None),
                })
                .collect(),
            _ => vec![(target, None)],
        },
        _ => Vec::new(),
    }
}

fn object_name<'a>(expr: &Expression<'a>) -> Option<&'a str> {
    match expr {
        Expression::Identifier(id) => Some(id.name.as_str()),
        _ => None,
    }
}

fn literal_key<'a>(expr: &Expression<'a>) -> Option<&'a str> {
    match expr {
        Expression::StringLiteral(literal) => literal.value.as_str(),
        Expression::TemplateLiteral(template) if template.expressions.is_empty() => {
            template.quasis.first().map(|quasi| quasi.value.raw.as_str())
        }
        _ => None,
    }
}

/// Whether a callee is written through a wrapper origin/main did not read (`component::main_peel`): the `Peeled` gate
/// every spelling-based match on a callee goes through.
pub(super) fn beyond_main(callee: &Expression<'_>) -> bool {
    super::component::peeled_beyond_main(callee)
}

/// Whether a receiver is `entry`: written plainly, by its spelling as origin/main read it; through a wrapper, only when
/// the name it is written with resolves to the entry ([`matches_well_known`]).
pub(super) fn is_receiver(scope: &dyn Scope, object: &Expression<'_>, entry: &WellKnown) -> bool {
    match object {
        Expression::Identifier(id) => id.name == entry.name,
        _ => matches!(super::component::beneath_resolved(object, scope),
            Some(Expression::Identifier(id)) if matches_well_known(scope, id.name.as_str(), entry)),
    }
}

/// The receiver name a member callee is matched with: written plainly, as origin/main read it; through a wrapper, only
/// when it resolves to one of `entries`; `""` otherwise, as origin/main named a receiver it could not read.
pub(super) fn receiver_among<'a>(
    scope: &dyn Scope,
    object: &Expression<'a>,
    beyond_main: bool,
    entries: &[&WellKnown],
) -> &'a str {
    match object {
        Expression::Identifier(id) if !beyond_main => id.name.as_str(),
        _ => match super::component::beneath_resolved(object, scope) {
            Some(Expression::Identifier(id)) if matches_any(scope, id.name.as_str(), entries) => id.name.as_str(),
            _ => "",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line of an extractor source outside this module and the test files that spells a well-known name as a
    /// string literal, with the reason it may: none of them matches a name read through a wrapper. A new line that
    /// spells one fails here; route it through [`matches_well_known`] or add it with its reason.
    const ALLOWED: &[(&str, &str, &str)] = &[
        ("mod.rs", "\"forwardRef\" => Some(Self::ForwardRef)", "maps an import's exported name, checked by react_api"),
        ("mod.rs", "\"memo\" => Some(Self::Memo)", "maps an import's exported name, checked by react_api"),
        ("mod.rs", "\"createContext\" => Some(Self::CreateContext)", "maps an import's exported name"),
        (
            "mod.rs",
            "if prefix == \"React\" {",
            "a type name's namespace (`React.FC`), not a value read through a wrapper",
        ),
        ("mod.rs", "None => (object == \"React\").then(|| member.to_owned()),", "a type name's namespace"),
        ("component.rs", "&& (prefix == \"React\"", "a type name's namespace in an annotation"),
        ("component.rs", "callee == \"memo\"", "the wrapper naming convention, read on main's callee"),
        ("evidence.rs", "if right == \"Promise\" {", "a type name (`Promise<Element>`), not a value"),
        ("evidence.rs", "right != \"Promise\")", "a type name, not a value"),
        ("interface.rs", "if matches!(wrapper, \"memo\" | \"forwardRef\") {", "only words a skip diagnostic"),
        ("visit.rs", "self.namespace_stack.push(\"React\".into());", "`declare namespace React` in a `.d.ts`"),
        (
            "written.rs",
            "\"Array\" => Some(WrittenMember::Kind(MemberKind::Array))",
            "a written type's name, not a value",
        ),
    ];

    #[test]
    fn no_extractor_source_spells_a_well_known_name_outside_this_module() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/extractor");
        let mut used = vec![false; ALLOWED.len()];
        let mut found = Vec::new();
        let mut files: Vec<_> = std::fs::read_dir(&dir).expect("the extractor sources").flatten().collect();
        files.sort_by_key(std::fs::DirEntry::file_name);
        for file in files {
            let name = file.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".rs") || name == "well_known.rs" || name.ends_with("tests.rs") {
                continue;
            }
            let source = std::fs::read_to_string(file.path()).expect("a source");
            for (number, line) in source.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                let spells = WELL_KNOWN.iter().any(|entry| line.contains(&format!("\"{}\"", entry.name)));
                if !spells {
                    continue;
                }
                match ALLOWED.iter().position(|(file, snippet, _)| *file == name && line.contains(snippet)) {
                    Some(index) => used[index] = true,
                    None => found.push(format!("{name}:{}: {}", number + 1, line.trim())),
                }
            }
        }
        assert!(found.is_empty(), "spelled outside well_known.rs:\n{}", found.join("\n"));
        let stale: Vec<_> = ALLOWED.iter().zip(&used).filter(|(_, used)| !**used).map(|(entry, _)| entry).collect();
        assert!(stale.is_empty(), "allow-list entries that match nothing: {stale:?}");
    }
}
