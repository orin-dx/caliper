#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A prop typed by TypeScript's own `Partial`, `Required`, `Pick`, `Omit` or `NonNullable` over a type whose members are
//! known reads as the type TypeScript computes (MUI's `classes?: Partial<AccordionClasses>`), not as the application;
//! `ReactElement` stays narrower than a node, and a union that holds a type parameter keeps its concrete members.

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use camino::Utf8PathBuf;

fn type_text(source: &str, prop: &str) -> String {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("Box.tsx"), source).unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.iter().find(|(key, _)| key.ends_with("Box")).expect("the component Box").1;
    entry.props[prop].prop_type.raw_string()
}

const PRELUDE: &str = r#"
/** The class keys. */
interface BaseClasses { /** Root. */ root: string; }
interface Classes extends BaseClasses { label: string; icon?: string | undefined }
type Transition = { mounted: boolean; duration?: number };
"#;

fn text_of(declared: &str) -> String {
    let source = format!("{PRELUDE}\nexport function Box(props: {{ value?: {declared} }}) {{ return null; }}");
    type_text(&source, "value")
}

#[test]
fn partial_of_an_interface_lists_its_members_and_its_parents_optional() {
    // The checker lists the inherited members first.
    assert_eq!(text_of("Partial<Classes>"), "{ root?: string; label?: string; icon?: string }");
}

#[test]
fn required_of_an_object_drops_undefined_from_optional_members() {
    assert_eq!(text_of("Required<Classes>"), "{ root: string; label: string; icon: string }");
}

#[test]
fn pick_lists_the_members_in_key_order_and_omit_filters_them_out() {
    assert_eq!(text_of("Pick<Classes, \"root\" | \"label\">"), "{ root: string; label: string }");
    assert_eq!(text_of("Partial<Omit<Transition, \"mounted\">>"), "{ duration?: number }");
}

#[test]
fn omit_of_an_array_by_the_keys_of_an_array_is_empty() {
    assert_eq!(text_of("Classes | Omit<Classes[], keyof any[]>"), "Classes | {  }");
}

#[test]
fn a_generic_interface_or_a_key_set_not_known_stays_an_application() {
    let source = format!(
        "{PRELUDE}\ninterface Slots<T> {{ root: T }}\nexport function Box(props: {{ a?: Partial<Slots<string>>; b?: Omit<Classes, string> }}) {{ return null; }}"
    );
    assert_eq!(type_text(&source, "a"), "Partial<Slots<string>>");
    assert_eq!(type_text(&source, "b"), "Omit<Classes, string>");
}

#[test]
fn non_nullable_of_a_known_type_drops_null_and_undefined() {
    assert_eq!(text_of("NonNullable<number | null | undefined>"), "number");
}

#[test]
fn a_react_element_stays_narrower_than_a_node() {
    let source = "import * as React from 'react';\nexport function Box(props: { icon: React.ReactElement; el: JSX.Element }) { return null; }";
    assert_eq!(type_text(source, "icon"), "ReactElement");
    assert_eq!(type_text(source, "el"), "JSX.Element");
}

#[test]
fn a_union_with_a_type_parameter_keeps_its_concrete_members() {
    let source = "import * as React from 'react';\ninterface ShowProps<T> { when: T | null | undefined }\nfunction Show<T>(props: ShowProps<T>) { return null; }\nexport function Box(props: React.ComponentProps<typeof Show>) { return null; }";
    assert_eq!(type_text(source, "when"), "unknown | null");
}

// ─── Exact or declined ───────────────────────────────────────────────────────
// The structural lane expands `Partial`, `Required`, `Pick` and `Omit` only when every fact the expansion reads is
// recorded; otherwise the application stays and the checker answers it. Each case below expanded to a wrong type.

/// Whether `text` is the application of `utility` left as written, not an expansion.
fn declined(text: &str, utility: &str) -> bool {
    text.starts_with(&format!("{utility}<"))
}

#[test]
fn omit_by_the_keys_of_an_index_signature_is_declined() {
    // TypeScript gives `{}`: `keyof { [k: string]: unknown }` is `string | number`, which takes every key away.
    let text = text_of("Omit<{ a: string }, keyof { [k: string]: unknown }>");
    assert!(declined(&text, "Omit"), "{text}");
}

#[test]
fn a_module_local_array_alias_is_not_the_builtin_array() {
    // `Array` here is the module's `{ value: T }`, whose keys an array's keys do not take away.
    let source = "type Array<T> = { value: T };\nexport function Box(props: { value?: Omit<Array<string>, keyof any[]> }) { return null; }";
    let text = type_text(source, "value");
    assert_ne!(text, "{  }");
}

#[test]
fn an_interface_with_an_index_signature_is_declined() {
    let source = "interface Dict { [k: string]: number }\nexport function Box(props: { value?: Partial<Dict> }) { return null; }";
    let text = type_text(source, "value");
    assert!(declined(&text, "Partial"), "{text}");
}

#[test]
fn partial_keeps_an_interface_members_readonly_modifier() {
    let source = "interface Config { readonly mode: string; size?: number }\nexport function Box(props: { value?: Partial<Config> }) { return null; }";
    assert_eq!(type_text(source, "value"), "{ readonly mode?: string; size?: number }");
}

#[test]
fn required_of_an_optional_undefined_member_is_never() {
    assert_eq!(text_of("Required<{ value?: undefined }>"), "{ value: never }");
}

/// The type of prop `prop` of component `component` in a project of `files`.
fn project_type(files: &[(&str, &str)], component: &str, prop: &str) -> String {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    for (name, text) in files {
        std::fs::write(root.join(name), text).unwrap();
    }
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.iter().find(|(key, _)| key.ends_with(component)).expect("the component").1;
    entry.props[prop].prop_type.raw_string()
}

const FIELD: (&str, &str) = ("Field.tsx", "import { make } from './factory';\nexport const Field = make();\n");

#[test]
fn a_factory_local_alias_shadows_the_module_interface_in_a_utility() {
    let factory = "interface Data { fromModule: string }\n\
         export function make() { type Data = { fromFunction: number }; return (p: { value?: Partial<Data> }) => null; }\n";
    let text = project_type(&[("factory.tsx", factory), FIELD], "Field", "value");
    assert_ne!(text, "{ fromModule?: string }");
}

#[test]
fn a_factory_local_partial_alias_is_not_the_lib_partial() {
    let factory = "interface Data { x: string }\n\
         export function make() { type Partial<T> = { wrapped: T }; return (p: { value?: Required<Partial<Data>> }) => null; }\n";
    let text = project_type(&[("factory.tsx", factory), FIELD], "Field", "value");
    assert_ne!(text, "{ x: string }");
}

#[test]
fn keys_through_a_local_alias_are_read_in_its_scope() {
    // `K` is the keys of the factory's own `Data`, so `Omit` takes them all away.
    let factory = "interface Data { fromModule: string }\n\
         export function make() { interface Data { fromFunction: number; r?: string } type K = keyof Data; return (p: { value?: Omit<Data, K> }) => null; }\n";
    let text = project_type(&[("factory.tsx", factory), FIELD], "Field", "value");
    assert!(text == "{  }" || declined(&text, "Omit"), "{text}");
}

#[test]
fn a_type_argument_from_another_file_is_read_in_that_files_module_scope() {
    // The factory reads its props in its own body's scope; `Foo` in the argument is `use.tsx`'s module `Foo`, not the
    // function-local one at the same position in `use.tsx`.
    let factory = "export function make<T>() { return (p: { value?: T }) => null; }\n";
    let used = "import { make } from './factory';\ninterface Foo { m: string }\n\
         function other() { interface Foo { l: number } return null; }\n\
         export const Field = make<Partial<Foo>>();\n";
    let text = project_type(&[("factory.tsx", factory), ("use.tsx", used)], "Field", "value");
    assert_ne!(text, "{ l?: number }");
}

#[test]
fn pick_with_keys_out_of_member_order_is_declined() {
    // The checker orders the members by when each key's literal type was created, which is not recorded.
    let text = text_of("Pick<{ a: string; b: number }, \"b\" | \"a\">");
    assert!(declined(&text, "Pick"), "{text}");
}

#[test]
fn a_lone_getter_is_a_readonly_member_under_a_utility() {
    let source = "interface Sized { get size(): number }\nexport function Box(props: { value?: Partial<Sized> }) { return null; }";
    assert_eq!(type_text(source, "value"), "{ readonly size?: number }");
}

#[test]
fn a_factory_local_interface_shadows_the_module_one_in_a_utility() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(
        root.join("factory.tsx"),
        "interface Data { fromModule: string }\n\
         export function make() { interface Data { fromFunction: number } return (p: { value?: Partial<Data> }) => null; }\n",
    )
    .unwrap();
    std::fs::write(root.join("Field.tsx"), "import { make } from './factory';\nexport const Field = make();\n")
        .unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.iter().find(|(key, _)| key.ends_with("Field")).expect("the component Field").1;
    assert_eq!(entry.props["value"].prop_type.raw_string(), "{ fromFunction?: number }");
}
