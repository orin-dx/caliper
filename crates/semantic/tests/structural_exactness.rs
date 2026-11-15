#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! Structural expansion is exact or declined, with the linked checker as the oracle. A generator writes `Partial`,
//! `Required`, `Pick` and `Omit`, alone and nested, over adversarial objects: index signatures, `readonly` members,
//! optional members typed `undefined` or an alias holding it, inherited members, a module-local `Array` and `Partial`
//! that shadow the builtins, and a factory-local interface that shadows a module one. Wherever the structural pass
//! expands an application to an object, the checker must give the same type; everywhere else the application stays as
//! written, for the checker to answer.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::{ExtractionOutput, PropType};
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

/// The objects every utility is applied to, declared in `lib.tsx`.
const DECLARATIONS: &str = r#"
interface Plain { a: string; b?: number }
interface Ro { readonly a: string; b?: number | undefined }
interface Dict { [k: string]: number }
interface Mixed { a: string; [k: string]: string }
interface Child extends Plain { readonly c: boolean }
type Maybe = string | undefined;
interface Aliased { a?: Maybe; b: boolean }
type Undef = { a?: undefined; b?: string | undefined };
type Lit = { readonly a: string; b?: number };
type Indexed = { a: string; [k: string]: unknown };
"#;

/// The bases the generator applies the utilities to.
const BASES: &[&str] =
    &["Plain", "Ro", "Dict", "Mixed", "Child", "Aliased", "Undef", "Lit", "Indexed", "{ value?: undefined }"];

/// Each utility over a base `B`, alone and nested.
const SHAPES: &[&str] = &[
    "Partial<B>",
    "Required<B>",
    "Pick<B, \"a\">",
    "Omit<B, \"a\">",
    "Omit<B, keyof Plain>",
    "Omit<B, keyof { [k: string]: unknown }>",
    "Partial<Required<B>>",
    "Required<Partial<B>>",
    "Pick<Partial<B>, \"a\">",
    "Omit<Required<B>, \"b\">",
    "Partial<Omit<B, \"a\">>",
    "Readonly<Partial<B>>",
];

/// Files whose names shadow a builtin or a module-level declaration.
const SHADOWS: &[(&str, &str)] = &[
    (
        "array.tsx",
        "type Array<T> = { value: T };\nexport function ArrayBox(props: { a?: Omit<Array<string>, keyof any[]>; b?: Omit<string[], keyof any[]> }) { return null; }\n",
    ),
    (
        "partial.tsx",
        "type Partial<T> = { wrapped: T };\ninterface Plain { a: string }\nexport function PartialBox(props: { a?: Partial<Plain> }) { return null; }\n",
    ),
    (
        "factory.tsx",
        "interface Data { fromModule: string }\n\
         export function make() {\n\
           interface Data { fromFunction: number; readonly r?: string }\n\
           return (p: { a?: Partial<Data>; b?: Required<Data>; c?: Omit<Data, \"r\">; d?: Pick<Partial<Data>, \"r\"> }) => null;\n\
         }\n",
    ),
    ("field.tsx", "import { make } from './factory';\nexport const Field = make();\n"),
];

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

fn project() -> (Project, Vec<String>) {
    let dir = tempfile::Builder::new().prefix("Exactness").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let mut props = Vec::new();
    let mut source = String::from(DECLARATIONS);
    source.push_str("export function Box(props: {\n");
    for base in BASES {
        for shape in SHAPES {
            let name = format!("p{}", props.len());
            source.push_str(&format!("  {name}?: {};\n", shape.replace('B', base)));
            props.push(name);
        }
    }
    source.push_str("}) { return null; }\n");
    std::fs::write(root.join("lib.tsx"), source).unwrap();
    for (file, text) in SHADOWS {
        std::fs::write(root.join(file), text).unwrap();
    }
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true, "jsx": "react-jsx", "module": "esnext", "moduleResolution": "bundler" } }"#,
    )
    .unwrap();
    (Project { _dir: dir, root }, props)
}

fn structural(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions { src_dirs: vec![project.root.clone()], ..PipelineOptions::default() })
}

/// Whether the structural pass expanded the application: it gave an object, not the application as written.
fn expanded(ty: &PropType) -> bool {
    matches!(ty, PropType::Object(_))
}

#[test]
fn every_structural_expansion_equals_the_checker_and_the_rest_declines() {
    let (project, _) = project();
    let output = structural(&project);
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(&output));
    request.request_timeout = Duration::from_secs(60);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(resolve(request)).ok());
    let outcome = receiver.recv_timeout(Duration::from_secs(120)).expect("the conversion ends");
    assert_eq!(outcome.diagnostics, []);

    let mut compared = 0;
    let mut declined = 0;
    let mut wrong = Vec::new();
    for (key, entry) in &output.components {
        let answer = outcome
            .components
            .iter()
            .find(|component| &component.key == key)
            .unwrap_or_else(|| panic!("the checker answers {key}"));
        for (name, prop) in &entry.props {
            let structural = prop.prop_type.raw_string();
            // The module's own `Partial` is an alias the resolver reads as any alias, not the lib type expanded.
            if key == "PartialBox" {
                assert_ne!(structural, "{ a?: string }", "the module's `Partial` is not TypeScript's");
                continue;
            }
            if !expanded(&prop.prop_type) {
                declined += 1;
                continue;
            }
            compared += 1;
            let checker = answer
                .props
                .iter()
                .find(|candidate| &candidate.name == name)
                .unwrap_or_else(|| panic!("the checker gives {key}.{name}"))
                .prop_type
                .raw_string();
            if structural != checker {
                wrong.push(format!("{key}.{name}: structural `{structural}`, checker `{checker}`"));
            }
        }
    }
    eprintln!("{compared} expansions compared with the checker, {declined} declined");
    assert!(wrong.is_empty(), "expansions the checker disagrees with:\n{}", wrong.join("\n"));
    assert!(compared > 40, "the generator exercises the expansion ({compared} compared)");
}
