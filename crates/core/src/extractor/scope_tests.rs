//! The scope each collected declaration is recorded with.

use camino::Utf8Path;

use super::parse_file;
use crate::types::{DeclarationScope, ScopeKind, ScopeStep, SourceData};

const FILE: &str = "/test/scopes.tsx";

fn collect(source: &str) -> SourceData {
    parse_file(Utf8Path::new(FILE), source)
}

fn scope(steps: &[(ScopeKind, u32)]) -> DeclarationScope {
    DeclarationScope::new(steps.iter().map(|&(kind, ordinal)| ScopeStep { kind, ordinal }).collect())
}

fn key(name: &str) -> String {
    format!("{FILE}:{name}")
}

/// The scope of the one interface declared as `name`, whatever scope it sits in.
fn interface_scope<'d>(data: &'d SourceData, name: &str) -> Option<&'d DeclarationScope> {
    let (own, local) = (key(name), format!("{}@", key(name)));
    let mut found = data
        .interfaces
        .iter()
        .filter(|(key, _)| **key == own || key.starts_with(&local))
        .map(|(_, interface)| &interface.scope);
    let scope = found.next();
    assert!(found.next().is_none(), "more than one interface is declared as {name}");
    scope
}

#[test]
fn module_level_declarations_of_every_kind_are_in_the_module_scope() {
    let data = collect("interface I {}\ntype A = string;\nenum E { X }\nclass C {}");
    assert_eq!(interface_scope(&data, "I"), Some(&DeclarationScope::default()));
    assert_eq!(data.type_alias_scopes.get(&key("A")), Some(&DeclarationScope::default()));
    assert_eq!(data.enum_declarations.get(&key("E")), Some(&DeclarationScope::default()));
    assert_eq!(data.class_scopes.get(&key("C")), Some(&DeclarationScope::default()));
    assert!(interface_scope(&data, "I").unwrap().is_module());
}

#[test]
fn a_namespace_member_is_in_a_namespace_scope() {
    let data = collect("namespace N { interface I {} type A = string; enum E { X } class C {} }");
    let in_n = scope(&[(ScopeKind::Namespace, 0)]);
    assert_eq!(interface_scope(&data, "N.I"), Some(&in_n));
    assert_eq!(data.type_alias_scopes.get(&key("N.A")), Some(&in_n));
    assert_eq!(data.enum_declarations.get(&key("N.E")), Some(&in_n));
    assert_eq!(data.class_scopes.get(&key("N.C")), Some(&in_n));
}

#[test]
fn a_function_local_declaration_is_in_a_function_body_scope() {
    let data = collect("function f() { interface I {} type A = string; class C {} }");
    let in_f = scope(&[(ScopeKind::FunctionBody, 0)]);
    assert_eq!(interface_scope(&data, "I"), Some(&in_f));
    assert_eq!(data.type_alias_scopes.get(&key("A")), Some(&in_f));
    assert_eq!(data.class_scopes.get(&key("C")), Some(&in_f));
}

#[test]
fn an_arrow_function_body_and_a_method_body_are_function_body_scopes() {
    let data = collect("const f = () => { type A = string; };\nclass K { m() { interface I {} } }");
    assert_eq!(data.type_alias_scopes.get(&key("A")), Some(&scope(&[(ScopeKind::FunctionBody, 0)])));
    assert_eq!(interface_scope(&data, "I"), Some(&scope(&[(ScopeKind::FunctionBody, 1)])));
}

#[test]
fn a_block_declaration_is_in_a_block_scope_nested_in_its_function() {
    let data = collect("function f() { if (a) { interface I {} } }\n{ type A = string; }");
    assert_eq!(interface_scope(&data, "I"), Some(&scope(&[(ScopeKind::FunctionBody, 0), (ScopeKind::Block, 0)])));
    assert_eq!(data.type_alias_scopes.get(&key("A")), Some(&scope(&[(ScopeKind::Block, 1)])));
}

#[test]
fn a_switch_case_declaration_is_in_a_switch_case_scope() {
    let data = collect("switch (x) { case 1: interface I {} break; default: type A = string; }");
    assert_eq!(interface_scope(&data, "I"), Some(&scope(&[(ScopeKind::SwitchCase, 0)])));
    assert_eq!(data.type_alias_scopes.get(&key("A")), Some(&scope(&[(ScopeKind::SwitchCase, 1)])));
}

#[test]
fn the_same_name_in_different_namespaces_has_different_scopes() {
    let data = collect("namespace A { interface X {} }\nnamespace B { interface X {} }");
    let (in_a, in_b) = (interface_scope(&data, "A.X").unwrap(), interface_scope(&data, "B.X").unwrap());
    assert_eq!(in_a, &scope(&[(ScopeKind::Namespace, 0)]));
    assert_eq!(in_b, &scope(&[(ScopeKind::Namespace, 1)]));
    assert_ne!(in_a, in_b);
}

#[test]
fn the_same_name_at_module_level_and_in_a_namespace_has_different_scopes() {
    let data = collect("type X = string;\nnamespace N { type X = number; }");
    assert_ne!(data.type_alias_scopes[&key("X")], data.type_alias_scopes[&key("N.X")]);
}

#[test]
fn sibling_blocks_get_different_ordinals_and_the_entry_keeps_the_last_declaration_scope() {
    // All three aliases share one key, so `type_aliases` holds the last; the scope recorded is the last one's too.
    let data = collect("type X = string;\n{ type X = number; }\n{ type X = boolean; }");
    assert_eq!(data.type_aliases.len(), 1);
    assert_eq!(data.type_alias_scopes.len(), 1);
    assert_eq!(data.type_alias_scopes[&key("X")], scope(&[(ScopeKind::Block, 1)]));
}

#[test]
fn a_local_interface_beside_a_module_level_one_is_an_entry_of_its_own() {
    let data = collect("interface X { a: string }\nfunction f() { interface X { b: string } }");
    let mut scopes: Vec<(&str, &DeclarationScope)> =
        data.interfaces.iter().map(|(key, interface)| (key.as_str(), &interface.scope)).collect();
    scopes.sort_by_key(|(key, _)| *key);
    assert_eq!(
        scopes,
        [
            ("/test/scopes.tsx:X", &DeclarationScope::default()),
            ("/test/scopes.tsx:X@f0", &scope(&[(ScopeKind::FunctionBody, 0)])),
        ]
    );
}

#[test]
fn a_local_enum_is_not_collected_so_it_has_no_scope() {
    let data = collect("function f() { enum E { X } }\n{ enum F { Y } }");
    assert!(data.enum_declarations.is_empty(), "got {:?}", data.enum_declarations);
}

#[test]
fn an_alias_the_extractor_synthesizes_has_no_scope() {
    let data = collect("export const B = (props: { a: string }) => <div />;");
    assert!(!data.type_aliases.is_empty(), "the inline props type is synthesized as an alias");
    assert!(data.type_alias_scopes.is_empty(), "got {:?}", data.type_alias_scopes);
}
