#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! One minimal source per declaration shape, asserting the exact component set and exact props, plus the
//! near misses that must not become components. Written from the shapes measured on Mantine, Chakra, Ant Design,
//! shadcn/ui and react-spectrum, so a recall regression on a shape fails here before a corpus run would show it.

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::Utf8PathBuf;

struct Case {
    shape: &'static str,
    source: &'static str,
    /// Component name to its exact prop names, sorted.
    expect: &'static [(&'static str, &'static [&'static str])],
}

const CASES: &[Case] = &[
    Case {
        shape: "forwardRef with explicit type arguments",
        source: r#"
            import { forwardRef } from "react";
            interface P { label?: string }
            export const A = forwardRef<HTMLButtonElement, P>((props, ref) => <button ref={ref} />);
        "#,
        expect: &[("A", &["label"])],
    },
    Case {
        shape: "FC annotation",
        source: r#"
            import type { FC } from "react";
            interface P { label?: string }
            export const A: FC<P> = ({ label }) => <b>{label}</b>;
        "#,
        expect: &[("A", &["label"])],
    },
    Case {
        shape: "function declaration",
        source: r#"
            interface P { label?: string }
            export function A(props: P) { return <b />; }
        "#,
        expect: &[("A", &["label"])],
    },
    Case {
        shape: "typed arrow function",
        source: r#"
            interface P { label?: string }
            export const A = ({ label }: P) => <b>{label}</b>;
        "#,
        expect: &[("A", &["label"])],
    },
    Case {
        shape: "typed function expression",
        source: r#"
            interface P { label?: string }
            export const A = function (props: P) { return <b />; };
        "#,
        expect: &[("A", &["label"])],
    },
    Case {
        shape: "memo around forwardRef",
        source: r#"
            import { forwardRef, memo } from "react";
            interface P { label?: string }
            export const A = memo(forwardRef<HTMLDivElement, P>((props, ref) => <div ref={ref} />));
        "#,
        expect: &[("A", &["label"])],
    },
    Case {
        shape: "parenthesized and non-null wrappers",
        source: r#"
            import { forwardRef } from "react";
            interface P { label?: string }
            export const A = (forwardRef<HTMLDivElement, P>((props, ref) => <div ref={ref} />));
            export const B = forwardRef<HTMLDivElement, P>((props, ref) => <div ref={ref} />)!;
        "#,
        expect: &[("A", &["label"]), ("B", &["label"])],
    },
    Case {
        shape: "factory call with a payload type argument",
        source: r#"
            import { factory } from "./lib";
            interface ButtonProps { label?: string }
            type ButtonFactory = Factory<{ props: ButtonProps; ref: HTMLButtonElement }>;
            export const Button = factory<ButtonFactory>((_props, ref) => <button ref={ref} />);
        "#,
        expect: &[("Button", &["label"])],
    },
    Case {
        shape: "factory payload declared after its use",
        source: r#"
            import { factory } from "./lib";
            export const Button = factory<ButtonFactory>(() => null);
            type ButtonFactory = Factory<{ props: ButtonProps }>;
            interface ButtonProps { label?: string }
        "#,
        expect: &[("Button", &["label"])],
    },
    Case {
        shape: "context call with an element type and a props type",
        source: r#"
            import { withContext } from "./lib";
            interface ItemProps { item?: string }
            export const Item = withContext<HTMLDivElement, ItemProps>("div", "item");
        "#,
        expect: &[("Item", &["item"])],
    },
    Case {
        shape: "props extended through a barrel",
        source: r#"
            import { forwardRef } from "react";
            import type { Shared } from "./barrel";
            interface P extends Shared { own?: string }
            export const A = forwardRef<HTMLDivElement, P>((props, ref) => <div ref={ref} />);
        "#,
        expect: &[("A", &["own", "shared"])],
    },
    Case {
        shape: "props extended through a barrel that imports then exports",
        source: r#"
            import { forwardRef } from "react";
            import type { Imported } from "./barrel2";
            interface P extends Imported { own?: string }
            export const A = forwardRef<HTMLDivElement, P>((props, ref) => <div ref={ref} />);
        "#,
        expect: &[("A", &["imported", "own"])],
    },
    Case {
        shape: "Pick with a generic base",
        source: r#"
            interface Gen<T> { x: T; y: string }
            export function A(props: Pick<Gen<number>, "x">) { return null; }
        "#,
        expect: &[("A", &["x"])],
    },
    Case {
        shape: "Omit of keyof another type",
        source: r#"
            interface Base { a: number; b: number }
            interface Other { b: number }
            export function A(props: Omit<Base, keyof Other>) { return null; }
        "#,
        expect: &[("A", &["a"])],
    },
    Case {
        shape: "generic interface with its own extends clause",
        source: r#"
            interface Box<T> { value: T }
            interface Wide<T> extends Box<T[]> { more: T }
            export function A(props: Wide<boolean>) { return null; }
        "#,
        expect: &[("A", &["more", "value"])],
    },
    // ── near misses that are not components ──────────────────────────────────
    Case {
        shape: "createContext with a default value",
        source: r#"
            import { createContext } from "react";
            interface ThemeProps { mode?: string }
            export const ThemeContext = createContext<ThemeProps>({ mode: "light" });
            export const NullContext = createContext<ThemeProps | null>(null);
        "#,
        expect: &[],
    },
    Case {
        shape: "generic call on a constant or with no argument",
        source: r#"
            interface ItemProps { item?: string }
            export const Limit = makeLimit<ItemProps>(42);
            export const Empty = makeThing<ItemProps>();
            export const Options = makeThing<ItemProps>({ a: 1 });
        "#,
        expect: &[],
    },
    Case {
        shape: "generic call naming only an element type",
        source: r#"
            export const Node = makeThing<HTMLDivElement>("div");
        "#,
        expect: &[],
    },
    Case {
        shape: "wrapper call whose props live in an uninstalled package",
        source: r#"
            import type { MissingProps } from "./missing";
            export const Ghost = wrap<HTMLDivElement, MissingProps>("div");
        "#,
        expect: &[("Ghost", &[])],
    },
    Case {
        shape: "generic call that is a context, store, label or hook",
        source: r#"
            import { createContext } from "react";
            interface ButtonProps { label?: string }
            interface BearState { bears: number }
            export const ButtonContext = createContext<ButtonProps>(defaultButton);
            export const MadeContext = createContext<ButtonProps>(makeDefault());
            export const MaybeContext = createContext<ButtonProps | undefined>(undefined);
            export const Bears = create<BearState>((set) => ({ bears: 0 }));
            export const ButtonLabel = makeLabel<ButtonProps>("button");
            export const ButtonSchema = z.object<ButtonProps>(shape);
            export const UseButton = makeHook<ButtonProps>(fetcher);
        "#,
        expect: &[],
    },
    Case {
        shape: "typed helpers and higher-order calls that never render",
        source: r#"
            interface User { first: string }
            export const FormatName = (u: User) => u.first;
            export function FullName(u: User): string { return u.first; }
            export function* Saga(u: User) { yield u; }
            export const Search = debounce((q: User) => q.first, 100);
            export const Picked = createSelector((u: User) => u.first);
            export const Memoized = memo((u: User) => u.first);
        "#,
        expect: &[],
    },
    Case {
        shape: "a component passed to a call that is not a wrapper",
        source: r#"
            interface CardProps { title?: string }
            export function Card(props: CardProps) { return <div />; }
            export const CardName = getDisplayName(Card);
            export const MemoCard = memo(Card);
        "#,
        expect: &[("Card", &["title"]), ("MemoCard", &["title"])],
    },
    Case {
        shape: "untyped and non-PascalCase functions",
        source: r#"
            interface P { label?: string }
            export const Untyped = (props) => <div />;
            export const Sum = (a: number, b: number) => a + b;
            export const helper = (props: P) => props.label;
        "#,
        expect: &[],
    },
    Case {
        shape: "async server component",
        source: r#"
            interface P { label?: string }
            export const Load = async (props: P) => <div>{props.label}</div>;
            export async function Page(props: P): Promise<JSX.Element> { return <main />; }
        "#,
        expect: &[("Load", &["label"]), ("Page", &["label"])],
    },
    Case {
        shape: "components that render without JSX",
        source: r#"
            import { createElement } from "react";
            interface P { children?: string }
            export const Passthrough = ({ children }: P) => children;
            export const Created = (props: P) => createElement("div", props);
        "#,
        expect: &[("Created", &["children"]), ("Passthrough", &["children"])],
    },
];

/// Extra files some cases import from, written next to every case's source.
const SUPPORT: &[(&str, &str)] = &[
    ("barrel/index.ts", "export type { Shared } from \"./shared\";\n"),
    ("barrel/shared.ts", "export interface Shared { shared?: string }\n"),
    ("barrel2/index.ts", "import type { Imported } from \"./shared\";\nexport type { Imported };\n"),
    ("barrel2/shared.ts", "export interface Imported { imported?: string }\n"),
];

fn run(case: &Case) -> BTreeMap<String, Vec<String>> {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    for (name, body) in SUPPORT {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    // `./barrel` in the case source resolves from the case file's directory.
    std::fs::write(
        root.join("case.tsx"),
        case.source.replace("\"./barrel\"", "\"./barrel/index\"").replace("\"./barrel2\"", "\"./barrel2/index\""),
    )
    .unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    extract(&options)
        .components
        .into_iter()
        .map(|(name, entry)| (name, entry.props.keys().cloned().collect()))
        .collect()
}

#[test]
fn every_declaration_shape_yields_exactly_the_expected_components_and_props() {
    let mut failures = Vec::new();
    for case in CASES {
        let got = run(case);
        let want: BTreeMap<String, Vec<String>> = case
            .expect
            .iter()
            .map(|(name, props)| ((*name).to_owned(), props.iter().map(|p| (*p).to_owned()).collect()))
            .collect();
        if got != want {
            failures.push(format!("{}:\n    want {want:?}\n    got  {got:?}", case.shape));
        }
    }
    assert!(failures.is_empty(), "shape regressions:\n{}", failures.join("\n"));
}
