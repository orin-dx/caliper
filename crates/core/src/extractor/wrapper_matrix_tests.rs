//! Every component detection form, crossed with every wrapper TypeScript can put around the expression it reads,
//! extracts the same component as the unwrapped form while the wrapper keeps the callee's type, and what the replacing
//! assertion says when it does not (`Expect`). OXC 0.159 stopped flattening `(f<T>)(x)` and `extends (B<T>)` into the
//! enclosing call or heritage clause, which is how a wrapper first broke detection.

use super::*;

const HEADER: &str = "import React, { forwardRef, memo, Component, PureComponent } from 'react';\n\
                      interface Props { label?: string }\n";

/// A wrapper around a callee or `extends` target `base` and its type arguments `args`, as written in source.
#[derive(Clone, Copy, Debug)]
enum Callee {
    Plain,
    Parenthesized,
    /// `(f<T>)`: the instantiation OXC 0.159 keeps as a node of its own.
    InstantiationInParens,
    InstantiationInDoubleParens,
    As,
    Satisfies,
    NonNull,
    InstantiationNonNull,
    /// `(<typeof f>f)<T>`: only valid where JSX is not.
    TypeAssertion,
    /// `(f<T> satisfies G)`: `satisfies` keeps the type of what it checks, so the type arguments still name the props.
    InstantiationSatisfies,
    /// `(React).forwardRef`: the namespace a member is read from is wrapped, not the callee. A bare name has none.
    ObjectParenthesized,
    ObjectAs,
    ObjectNonNull,
    /// `(f as any)`: `any` says nothing about the callee, so its name still does.
    AsAny,
    /// `(f as (...args: any[]) => object)`: the assertion states a result that is no component.
    AsObjectResult,
    /// `(f as unknown as (...args: any[]) => object)`: the same through `unknown`.
    AsUnknownAsObjectResult,
    /// `(f as (...args: any[]) => React.FC<…>)`: a named result the extractor does not resolve.
    AsOtherComponentResult,
    /// `(React as object).forwardRef`: the namespace is asserted to a type with no such member, which is no evidence.
    ObjectAsObject,
    /// `(<(...args: any[]) => object>f)`: the `<T>x` spelling of `AsObjectResult`, only valid where JSX is not.
    TypeAssertionObjectResult,
}

/// What a row extracts, against the unwrapped form.
#[derive(Debug, PartialEq)]
enum Expect {
    /// The assertion keeps the callee's type (none, `typeof` itself, `any`): the unwrapped form's component.
    Same,
    /// The assertion replaces the callee's type with one that is not read as no component: the name still says what
    /// the call is, but the callee's own declaration no longer describes it, so the mapping has no wrapper callee.
    SameWithoutCallee,
    /// The assertion states a result no component or element has: nothing is read through the callee's name.
    Declined,
}

impl Callee {
    const ALL: [Self; 19] = [
        Self::Plain,
        Self::Parenthesized,
        Self::InstantiationInParens,
        Self::InstantiationInDoubleParens,
        Self::As,
        Self::Satisfies,
        Self::NonNull,
        Self::InstantiationNonNull,
        Self::TypeAssertion,
        Self::InstantiationSatisfies,
        Self::ObjectParenthesized,
        Self::ObjectAs,
        Self::ObjectNonNull,
        Self::AsAny,
        Self::AsObjectResult,
        Self::AsUnknownAsObjectResult,
        Self::AsOtherComponentResult,
        Self::ObjectAsObject,
        Self::TypeAssertionObjectResult,
    ];

    fn expect(self) -> Expect {
        match self {
            Self::Plain
            | Self::Parenthesized
            | Self::InstantiationInParens
            | Self::InstantiationInDoubleParens
            | Self::As
            | Self::Satisfies
            | Self::NonNull
            | Self::InstantiationNonNull
            | Self::TypeAssertion
            | Self::InstantiationSatisfies
            | Self::ObjectParenthesized
            | Self::ObjectAs
            | Self::ObjectNonNull
            | Self::AsAny => Expect::Same,
            // A namespace asserted to a type that declares no such member says nothing the extractor reads.
            Self::AsOtherComponentResult | Self::ObjectAsObject => Expect::SameWithoutCallee,
            Self::AsObjectResult | Self::AsUnknownAsObjectResult | Self::TypeAssertionObjectResult => Expect::Declined,
        }
    }

    fn wrap(self, base: &str, args: &str) -> String {
        let (object, member) = base.split_once('.').map_or((None, base), |(object, member)| (Some(object), member));
        match self {
            Self::Plain => format!("{base}{args}"),
            Self::Parenthesized => format!("({base}){args}"),
            Self::InstantiationInParens => format!("({base}{args})"),
            Self::InstantiationInDoubleParens => format!("(({base}{args}))"),
            Self::As => format!("({base} as typeof {base}){args}"),
            Self::Satisfies => format!("({base} satisfies typeof {base}){args}"),
            Self::NonNull => format!("{base}!{args}"),
            Self::InstantiationNonNull => format!("({base}{args})!"),
            Self::TypeAssertion => format!("(<typeof {base}>{base}){args}"),
            Self::InstantiationSatisfies => format!("({base}{args} satisfies typeof {base})"),
            Self::ObjectParenthesized => {
                object.map_or_else(|| format!("{base}{args}"), |o| format!("({o}).{member}{args}"))
            }
            Self::ObjectAs => {
                object.map_or_else(|| format!("{base}{args}"), |o| format!("({o} as any).{member}{args}"))
            }
            Self::ObjectNonNull => object.map_or_else(|| format!("{base}{args}"), |o| format!("{o}!.{member}{args}")),
            Self::AsAny => format!("({base} as any){args}"),
            Self::AsObjectResult => format!("({base} as (...args: any[]) => object){args}"),
            Self::AsUnknownAsObjectResult => format!("({base} as unknown as (...args: any[]) => object){args}"),
            Self::AsOtherComponentResult => {
                format!("({base} as (...args: any[]) => React.FC<{{ other?: number }}>){args}")
            }
            Self::ObjectAsObject => {
                object.map_or_else(|| format!("{base}{args}"), |o| format!("({o} as object).{member}{args}"))
            }
            Self::TypeAssertionObjectResult => format!("(<(...args: any[]) => object>{base}){args}"),
        }
    }
}

/// A wrapper around a whole initializer or default-exported expression.
#[derive(Clone, Copy, Debug)]
enum Value {
    Plain,
    Parenthesized,
    As,
    Satisfies,
    NonNull,
    TypeAssertion,
}

impl Value {
    const ALL: [Self; 6] =
        [Self::Plain, Self::Parenthesized, Self::As, Self::Satisfies, Self::NonNull, Self::TypeAssertion];

    fn wrap(self, value: &str) -> String {
        match self {
            Self::Plain => value.to_owned(),
            Self::Parenthesized => format!("({value})"),
            Self::As => format!("({value}) as React.ComponentType<Props>"),
            Self::Satisfies => format!("({value}) satisfies React.ComponentType<Props>"),
            Self::NonNull => format!("({value})!"),
            Self::TypeAssertion => format!("<React.ComponentType<Props>>({value})"),
        }
    }
}

const RENDER: &str = "React.createElement('button')";

/// A detection form: a source with one slot for a callee or heritage wrapper and one for a value wrapper.
struct Form {
    name: &'static str,
    /// The file it is read from; an anonymous default export is named after it.
    stem: &'static str,
    source: fn(Callee, Value) -> String,
}

fn forms() -> Vec<Form> {
    vec![
        Form {
            name: "React.forwardRef",
            stem: "Button",
            source: |c, v| {
                let call = format!(
                    "{}(({{ label = 'x' }}, ref) => {RENDER})",
                    c.wrap("React.forwardRef", "<HTMLButtonElement, Props>")
                );
                format!("export const Button = {};", v.wrap(&call))
            },
        },
        Form {
            name: "forwardRef",
            stem: "Button",
            source: |c, v| {
                let call = format!(
                    "{}(({{ label = 'x' }}, ref) => {RENDER})",
                    c.wrap("forwardRef", "<HTMLButtonElement, Props>")
                );
                format!("export const Button = {};", v.wrap(&call))
            },
        },
        Form {
            name: "forwardRef with a typed render function",
            stem: "Button",
            source: |c, v| {
                let call = format!(
                    "{}(function Button({{ label = 'x' }}: Props, ref) {{ return {RENDER}; }})",
                    c.wrap("forwardRef", "")
                );
                format!("export const Button = {};", v.wrap(&call))
            },
        },
        Form {
            name: "React.memo",
            stem: "Button",
            source: |c, v| {
                let call = format!("{}(({{ label = 'x' }}: Props) => {RENDER})", c.wrap("React.memo", "<Props>"));
                format!("export const Button = {};", v.wrap(&call))
            },
        },
        Form {
            name: "memo",
            stem: "Button",
            source: |c, v| {
                let call = format!("{}(({{ label = 'x' }}: Props) => {RENDER})", c.wrap("memo", ""));
                format!("export const Button = {};", v.wrap(&call))
            },
        },
        Form {
            name: "memo(forwardRef), wrapped inner callee",
            stem: "Button",
            source: |c, v| {
                let call = format!(
                    "memo({}(({{ label = 'x' }}, ref) => {RENDER}))",
                    c.wrap("React.forwardRef", "<HTMLButtonElement, Props>")
                );
                format!("export const Button = {};", v.wrap(&call))
            },
        },
        Form {
            name: "memo(forwardRef), wrapped outer callee",
            stem: "Button",
            source: |c, v| {
                let call = format!(
                    "{}(forwardRef<HTMLButtonElement, Props>(({{ label = 'x' }}, ref) => {RENDER}))",
                    c.wrap("React.memo", "")
                );
                format!("export const Button = {};", v.wrap(&call))
            },
        },
        Form {
            name: "class extends React.Component",
            stem: "Button",
            source: |c, _| {
                format!(
                    "export class Button extends {} {{ render() {{ return {RENDER}; }} }}",
                    c.wrap("React.Component", "<Props>")
                )
            },
        },
        Form {
            name: "class extends PureComponent",
            stem: "Button",
            source: |c, _| {
                format!(
                    "export class Button extends {} {{ render() {{ return {RENDER}; }} }}",
                    c.wrap("PureComponent", "<Props>")
                )
            },
        },
        Form {
            name: "class expression",
            stem: "Button",
            source: |c, v| {
                let class =
                    format!("class extends {} {{ render() {{ return {RENDER}; }} }}", c.wrap("Component", "<Props>"));
                format!("export const Button = {};", v.wrap(&class))
            },
        },
        Form {
            name: "arrow function",
            stem: "Button",
            source: |_, v| {
                format!("export const Button = {};", v.wrap(&format!("({{ label = 'x' }}: Props) => {RENDER}")))
            },
        },
        Form {
            name: "React.FC annotation",
            stem: "Button",
            source: |_, v| {
                let arrow = format!("({{ label = 'x' }}) => {RENDER}");
                format!("export const Button: React.FC<Props> = {};", v.wrap(&arrow))
            },
        },
        Form {
            name: "function expression",
            stem: "Button",
            source: |_, v| {
                let function = format!("function Button({{ label = 'x' }}: Props) {{ return {RENDER}; }}");
                format!("export const Button = {};", v.wrap(&function))
            },
        },
        Form {
            name: "export default React.forwardRef",
            stem: "Button",
            source: |c, v| {
                let call = format!(
                    "{}(({{ label = 'x' }}, ref) => {RENDER})",
                    c.wrap("React.forwardRef", "<HTMLButtonElement, Props>")
                );
                format!("export default {};", v.wrap(&call))
            },
        },
        Form {
            name: "export default memo",
            stem: "Button",
            source: |c, v| {
                let call = format!("{}(({{ label = 'x' }}: Props) => {RENDER})", c.wrap("memo", ""));
                format!("export default {};", v.wrap(&call))
            },
        },
        Form {
            name: "export default class",
            stem: "Button",
            source: |c, _| {
                format!(
                    "export default class Button extends {} {{ render() {{ return {RENDER}; }} }}",
                    c.wrap("React.Component", "<Props>")
                )
            },
        },
    ]
}

/// What a component's extraction is, without where it sits in the file: the wrappers move its span.
fn extracted(path: &Utf8Path, source: &str) -> (Vec<serde_json::Value>, Vec<String>) {
    let data = parse_file(path, &format!("{HEADER}{source}"));
    let parse_errors: Vec<String> = data
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::ParseError)
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    let mappings = data
        .component_mappings
        .iter()
        .map(|mapping| {
            let mut value = serde_json::to_value(mapping).unwrap();
            let object = value.as_object_mut().unwrap();
            object.remove("span_start");
            object.remove("span_end");
            value
        })
        .collect();
    (mappings, parse_errors)
}

/// An assertion replaces the type of the instantiation beneath it, so its type arguments no longer name the props.
#[test]
fn type_arguments_beneath_an_assertion_do_not_name_the_props() {
    for callee in ["(forwardRef<HTMLButtonElement, Props> as any)", "(<any>forwardRef<HTMLButtonElement, Props>)"] {
        let source = format!("export const Button = {callee}(({{ label = 'x' }}, ref) => {RENDER});");
        let (mappings, errors) = extracted(Utf8Path::new("/fixtures/Button.ts"), &source);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(mappings, Vec::<serde_json::Value>::new(), "{source}");
    }
}

/// The alias detection and the inline-member scan of `Object.assign(Impl, { Item })` answer one question: they agree
/// through every wrapper of the callee, of its namespace and of the first argument.
#[test]
fn object_assign_attaches_its_inline_members_through_every_wrapper() {
    let sources = |assign: &str, first: &str| {
        format!(
            "{HEADER}interface ItemProps {{ size?: string }}\n\
             function Impl(props: Props) {{ return {RENDER}; }}\n\
             export const Root = {assign}({first}, {{ Item: (props: ItemProps) => {RENDER} }});"
        )
    };
    let path = Utf8Path::new("/fixtures/Root.tsx");
    let (expected, errors) = extracted(path, &sources("Object.assign", "Impl"));
    assert!(errors.is_empty(), "{errors:?}");
    let names = |mappings: &[serde_json::Value]| -> Vec<String> {
        let mut names: Vec<String> =
            mappings.iter().filter_map(|mapping| mapping["component_name"].as_str().map(str::to_owned)).collect();
        names.sort();
        names
    };
    assert_eq!(names(&expected), ["Item", "Root"], "{expected:#?}");
    let mut failures = Vec::new();
    for assign in [
        "(Object).assign",
        "(Object as typeof Object).assign",
        "Object!.assign",
        "(Object satisfies ObjectConstructor).assign",
    ] {
        for first in ["Impl", "(Impl)", "Impl as typeof Impl", "Impl!"] {
            let source = sources(assign, first);
            let (actual, errors) = extracted(path, &source);
            if !errors.is_empty() || actual != expected {
                failures.push(format!("{assign}({first}, …): {errors:?}\n{source}\nactual {:?}", names(&actual)));
            }
        }
    }
    assert!(failures.is_empty(), "{} forms differ from the plain one:\n{}", failures.len(), failures.join("\n\n"));
}

#[test]
fn every_detection_form_extracts_the_same_component_through_every_wrapper() {
    let mut failures = Vec::new();
    for form in forms() {
        // A type assertion `<T>x` is JSX in a `.tsx` file; the `.ts` rows read it, and a `.ts` file importing React
        // is read for components.
        for extension in ["tsx", "ts"] {
            let path = Utf8PathBuf::from(format!("/fixtures/{}.{extension}", form.stem));
            let plain = (form.source)(Callee::Plain, Value::Plain);
            let (unwrapped, errors) = extracted(&path, &plain);
            assert!(errors.is_empty(), "{}: {errors:?}", form.name);
            let [mapping] = unwrapped.as_slice() else {
                panic!("{}: expected one component, got {unwrapped:#?}", form.name)
            };
            assert_eq!(mapping["component_name"], "Button", "{}", form.name);
            assert_eq!(mapping["props_type_name"], "Props", "{}", form.name);
            let rows = Callee::ALL
                .iter()
                .map(|&callee| (callee, Value::Plain))
                .chain(Value::ALL.iter().map(|&value| (Callee::Plain, value)));
            for (callee, value) in rows {
                let jsx_only = extension == "tsx";
                if jsx_only
                    && (matches!(callee, Callee::TypeAssertion | Callee::TypeAssertionObjectResult)
                        || matches!(value, Value::TypeAssertion))
                {
                    continue;
                }
                let source = (form.source)(callee, value);
                let (actual, errors) = extracted(&path, &source);
                // A form that reads no callee is written the same whatever the callee wrapper.
                let expect = if source == plain { Expect::Same } else { callee.expect() };
                let expected: Vec<serde_json::Value> = match expect {
                    Expect::Same => unwrapped.clone(),
                    Expect::SameWithoutCallee => unwrapped
                        .iter()
                        .map(|mapping| {
                            let mut mapping = mapping.clone();
                            mapping["wrapper_callee"] = serde_json::Value::Null;
                            mapping
                        })
                        .collect(),
                    Expect::Declined => Vec::new(),
                };
                if !errors.is_empty() {
                    failures.push(format!(
                        "{} [{callee:?}, {value:?}] .{extension}: parse errors {errors:?}\n{source}",
                        form.name
                    ));
                } else if actual != expected {
                    failures.push(format!(
                        "{} [{callee:?}, {value:?}] .{extension}, {expect:?}:\n\
                         {source}\n\
                         expected {expected:#?}\n\
                         actual {actual:#?}",
                        form.name
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} rows differ from what they expect:\n\n{}", failures.len(), failures.join("\n\n"));
}

/// `Object.assign` through a cast that is no evidence against it makes `Root` an alias of `Impl`, and the inline
/// member scan reads the call through the same predicate, so `Item` is found with it. A namespace cast to `object`
/// declares no `assign`, which says nothing; a member declared to return `object` finds neither.
/// (`Impl` is not published under its own name while `Root` aliases it.)
#[test]
fn an_object_assign_through_an_identity_cast_attaches_its_inline_members() {
    let found = |object: &str| {
        let source = format!(
            "interface P {{ a: string }}\n\
             interface ItemProps {{ b: string }}\n\
             function Impl(props: P) {{ return <div />; }}\n\
             export const Root = {object}.assign(Impl, {{ Item: (props: ItemProps) => <div /> }});"
        );
        let data = parse_file(Utf8Path::new("/test/compound.tsx"), &source);
        let mut found: Vec<(String, String)> =
            data.component_mappings.iter().map(|m| (m.component_name.clone(), m.props_type_name.to_string())).collect();
        found.sort_unstable();
        found
    };
    let pair = |name: &str, props: &str| (name.to_owned(), props.to_owned());
    for object in ["Object", "(Object as typeof Object)", "(Object as any)", "(Object)!", "(Object as object)"] {
        assert_eq!(found(object), [pair("Item", "ItemProps"), pair("Root", "P")], "{object}");
    }
    // Not an alias, `Root` takes nothing and `Impl` stands as its own component.
    assert_eq!(found("(Object as { assign(...args: any[]): object })"), [pair("Impl", "P")]);
}

/// A PascalCase callee is rendering evidence only while its name describes the call: an assertion stating a result
/// that is no named type takes it away; a named or React result, `typeof` itself and `any` leave it.
/// A component invoked directly and returned gives this function's result, and a component may return a string, a
/// number, a boolean or `null`: only a cast to a result React cannot render is evidence that it does not render.
#[test]
fn an_asserted_callee_renders_only_when_the_assertion_leaves_a_named_result() {
    let rows = [
        ("(Object as (value: object) => object)", false),
        ("(Object as { (value: object): object })", false),
        ("(Object as (value: object) => symbol)", false),
        ("(Object as (value: object) => { id: string })", false),
        ("(Object as (value: object) => unknown[])", false),
        ("(Object as (value: object) => object | null)", false),
        ("(Object as unknown as (value: object) => string)", true),
        ("(Object as (value: object) => number)", true),
        ("(Object as (value: object) => boolean)", true),
        ("(Object as (value: object) => string | null)", true),
        ("(Object as (value: object) => null)", true),
        ("(Object as (value: object) => string | object)", true),
        ("(Object as (value: object) => string[])", true),
        ("(Object as (value: object) => React.ReactElement)", true),
        ("(Object as (value: object) => React.ReactElement | null)", true),
        ("(Object as F)", true),
        ("(Object as typeof Object)", true),
        ("(Object as any)", true),
        ("Object", true),
    ];
    for (callee, renders) in rows {
        let source = format!(
            "import React from 'react';\n\
             interface Props {{ a: string }}\n\
             export const Normalize = (props: Props) => {callee}(props);"
        );
        let data = parse_file(Utf8Path::new("/test/normalize.tsx"), &source);
        let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
        let expected: &[&str] = if renders { &["Normalize"] } else { &[] };
        assert_eq!(names, expected, "{callee}");
    }
}

/// The extraction of `source` after `HEADER` in a `.tsx` file, without spans.
fn tsx(source: &str) -> Vec<serde_json::Value> {
    let (mappings, errors) = extracted(Utf8Path::new("/fixtures/Button.tsx"), source);
    assert!(errors.is_empty(), "{errors:?}\n{source}");
    mappings
}

/// Review round 4, finding 1: a local alias of `typeof` the callee is the callee's own type, so its declaration still
/// gives the factory's result.
#[test]
fn a_factory_asserted_to_an_alias_of_its_own_type_is_read_through_its_declaration() {
    let factory = "function createButton(): React.FC<Props> { return (props) => <div />; }\n";
    let plain = tsx(&format!("{factory}export const Button = createButton();"));
    assert_eq!(plain.len(), 1, "{plain:#?}");
    assert_eq!(plain[0]["component_name"], "Button");
    for alias in ["type Factory = typeof createButton;", "export type Factory = (typeof createButton);"] {
        let source = format!("{factory}{alias}\nexport const Button = (createButton as Factory)();");
        assert_eq!(tsx(&source), plain, "{source}");
    }
    // An alias of another value's type, a generic alias and one declared twice are not the identity.
    for alias in [
        "declare const other: unknown; type Factory = typeof other;",
        "type Factory<T = unknown> = typeof createButton;",
        "type Factory = typeof createButton; namespace N {} type Factory = string;",
    ] {
        let source = format!("{factory}{alias}\nexport const Button = (createButton as Factory)();");
        assert_eq!(tsx(&source), Vec::<serde_json::Value>::new(), "{source}");
    }
}

/// Finding 2: only the outermost assertion gives the callee its type.
#[test]
fn only_the_outermost_assertion_decides() {
    let plain = tsx("export const Button = memo((props: Props) => <div />);");
    assert_eq!(plain.len(), 1, "{plain:#?}");
    assert_eq!(tsx("export const Button = (memo as object as typeof memo)((props: Props) => <div />);"), plain);
    assert_eq!(tsx("export const Button = ((memo as (c: any) => object) as any)((props: Props) => <div />);"), plain);
    let declined = "export const Button = (memo as typeof memo as (c: any) => object)((props: Props) => <div />);";
    assert_eq!(tsx(declined), Vec::<serde_json::Value>::new());
}

/// Finding 3: a namespace asserted to a type literal gives the member read from it the type the literal declares.
#[test]
fn a_namespace_assertion_gives_the_member_its_declared_type() {
    let render = "<HTMLButtonElement, Props>((props, ref) => <div />);";
    let plain = tsx(&format!("export const Button = React.forwardRef{render}"));
    assert_eq!(plain.len(), 1, "{plain:#?}");
    for receiver in [
        "(React as { forwardRef: typeof React.forwardRef })",
        "(React as { forwardRef: any })",
        "(React as typeof React)",
        "((React as object) as any)",
    ] {
        assert_eq!(tsx(&format!("export const Button = {receiver}.forwardRef{render}")), plain, "{receiver}");
    }
    for receiver in
        ["(React as { forwardRef: (...args: any[]) => object })", "(React as { forwardRef(...args: any[]): string })"]
    {
        let source = format!("export const Button = {receiver}.forwardRef{render}");
        assert_eq!(tsx(&source), Vec::<serde_json::Value>::new(), "{receiver}");
    }
}

/// Finding 4: a chain of assertions is read in one pass, whatever its length, and a deep one does not overflow.
#[test]
fn a_chain_of_assertions_is_read_in_linear_steps() {
    let steps = |n: usize| {
        let chain = " as typeof F".repeat(n);
        let source = format!(
            "{HEADER}function F(props: Props) {{ return <div />; }}\n\
             export const Button = (props: Props) => (F{chain})(props);"
        );
        super::component::steps::take();
        let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), &source);
        let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
        // `typeof F` is a reference to `F` that is no call, so `F` is not dropped as a helper.
        assert_eq!(names, ["F", "Button"], "{n}");
        super::component::steps::take()
    };
    let (ten, twenty, thirty) = (steps(10), steps(20), steps(30));
    assert_eq!(thirty - twenty, twenty - ten, "steps {ten}, {twenty}, {thirty}");
    assert!(thirty < 30 * 100, "{thirty} steps for 30 assertions");

    let deep = 2000;
    let nested = (0..deep).fold("F".to_owned(), |inner, _| format!("({inner} as typeof F)"));
    let parens = format!("{}F{}", "(".repeat(deep), ")".repeat(deep));
    // OXC's parser and visitor recurse once per level and a debug build's frames are large, so the thread gives them
    // room; the peel itself is a loop (`component::layers`).
    let deep_cases = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        for (callee, expected) in [(nested, &["F", "Button"][..]), (parens, &["Button"][..])] {
            let source = format!(
                "{HEADER}function F(props: Props) {{ return <div />; }}\n\
                 export const Button = (props: Props) => {callee}(props);"
            );
            let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), &source);
            let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
            assert_eq!(names, expected);
        }
    });
    deep_cases.unwrap().join().unwrap();
}

/// Finding 5: a call through an assertion is a call of the value beneath it, whatever the assertion says it returns,
/// so a helper only ever called stays a helper.
#[test]
fn a_call_through_an_assertion_counts_as_a_call_of_the_value() {
    let names = |call: &str| {
        let source = format!(
            "{HEADER}function Helper(props: Props) {{ return <div />; }}\n\
             export const Button = (props: Props) => {call}(props);"
        );
        let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), &source);
        let skipped: Vec<String> = data
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::SkippedCandidate && d.message.starts_with("'Helper'"))
            .map(|d| d.message.clone())
            .collect();
        let names: Vec<String> = data.component_mappings.iter().map(|m| m.component_name.clone()).collect();
        (names, skipped.len())
    };
    assert_eq!(names("Helper"), (vec!["Button".to_owned()], 1));
    assert_eq!(names("(Helper as (props: Props) => unknown)"), (vec!["Button".to_owned()], 1));
    // The result is no element, so `Button` does not render either; `Helper` is still only called.
    assert_eq!(names("(Helper as (props: Props) => object)"), (Vec::new(), 1));
}

/// Finding 6: a callable type literal is read for the results of its call signatures.
#[test]
fn a_callable_type_literal_is_read_for_its_call_signatures() {
    let plain = tsx("export const Button = memo((props: Props) => <div />);");
    assert_eq!(plain.len(), 1, "{plain:#?}");
    for ty in [
        "{ (render: React.FC<Props>): React.FC<Props> }",
        "{ (render: React.FC<Props>): React.FC<Props>; (render: unknown): string }",
        "{ (render: React.FC<Props>) }",
        "{ new (render: React.FC<Props>): object }",
    ] {
        assert_eq!(tsx(&format!("export const Button = (memo as {ty})((props: Props) => <div />);")), plain, "{ty}");
    }
    for ty in ["{ (render: React.FC<Props>): object }", "{ (a: unknown): string; (b: unknown): void }"] {
        let source = format!("export const Button = (memo as {ty})((props: Props) => <div />);");
        assert_eq!(tsx(&source), Vec::<serde_json::Value>::new(), "{ty}");
    }
}

/// Review round 5, finding 3: calling a hook is the evidence, whatever an assertion says the hook returns.
#[test]
fn a_hook_called_through_an_assertion_is_still_a_hook_call() {
    let names = |hook: &str| {
        let source = format!(
            "declare function useThing(): object;\n\
             export const Panel = (props: Props) => {{ {hook}(); return props.label; }};"
        );
        tsx(&source).iter().map(|m| m["component_name"].as_str().unwrap().to_owned()).collect::<Vec<_>>()
    };
    assert_eq!(names("useThing"), ["Panel"]);
    assert_eq!(names("(useThing as () => object)"), ["Panel"]);
    assert_eq!(names("(useThing as () => string)"), ["Panel"]);
    assert_eq!(names("(notAHook as () => object)"), Vec::<String>::new());
}

/// Finding 2: an initializer, a wrapper's argument and `Object.assign`'s first argument are read through an
/// instantiation beneath any other wrapper, as through the others: `memo(List<Row>)` is `List` memoized. The props the
/// type arguments give are pinned by `an_instantiated_component_binds_its_type_parameters_or_says_it_did_not`.
#[test]
fn an_instantiated_value_is_read_through_at_every_initializer_and_argument_site() {
    let prelude = "interface ListProps<T> { items?: T[] }\n\
                   interface ItemProps { size?: string }\n\
                   declare function withTheme<C>(c: C): C;\n\
                   function List<T>(props: ListProps<T>) { return <div />; }\n";
    let sites: [(&str, &[&str]); 4] = [
        ("export const Alias = {V};", &["Alias"]),
        ("export const Memo = memo({V});", &["Memo"]),
        ("export const Themed = withTheme({V});", &["Themed"]),
        ("export const Root = Object.assign({V}, { Item: (props: ItemProps) => <div /> });", &["Item", "Root"]),
    ];
    let sorted = |mappings: &[serde_json::Value]| {
        let mut names: Vec<String> =
            mappings.iter().map(|m| m["component_name"].as_str().unwrap().to_owned()).collect();
        names.sort();
        names
    };
    let mut failures = Vec::new();
    for (site, expected) in sites {
        let plain = tsx(&format!("{prelude}{}", site.replace("{V}", "List")));
        if sorted(&plain) != expected {
            failures.push(format!("{site} plain: {:?}", sorted(&plain)));
        }
        for value in [
            "List<string>",
            "(List<string>)",
            "(List<string>) as any",
            "(List<string>)!",
            "(List<string>) satisfies unknown",
            "((List as any)<string>)!",
        ] {
            let actual = tsx(&format!("{prelude}{}", site.replace("{V}", value)));
            if sorted(&actual) != sorted(&plain) {
                failures.push(format!("{site} with {value}: {:?}", sorted(&actual)));
            }
        }
    }
    // An initializer's function under an instantiation keeps its parameter defaults.
    let annotated = |arrow: &str| tsx(&format!("export const Button: React.FC<Props> = {arrow};"));
    let plain = annotated("<T,>({ label = 'x' }) => <div />");
    assert_eq!(plain[0]["param_defaults"]["label"]["value"], "'x'", "{plain:#?}");
    for arrow in
        ["((<T,>({ label = 'x' }) => <div />)<string>)!", "((<T,>({ label = 'x' }) => <div />)<string>) as any"]
    {
        if annotated(arrow) != plain {
            failures.push(format!("React.FC annotation with {arrow}: {:#?}", annotated(arrow)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Finding 5: a type literal's member has every type declared for its name, so it declines only when each one does.
#[test]
fn a_namespace_member_declared_several_times_declines_only_when_every_declaration_does() {
    let render = "<HTMLButtonElement, Props>((props, ref) => <div />);";
    let plain = tsx(&format!("export const Button = React.forwardRef{render}"));
    assert_eq!(plain.len(), 1, "{plain:#?}");
    let with = |literal: &str| tsx(&format!("export const Button = (React as {literal}).forwardRef{render}"));
    for literal in [
        "{ forwardRef(...a: any[]): string; forwardRef(...a: any[]): React.FC<Props> }",
        "{ forwardRef: (...a: any[]) => string; forwardRef(...a: any[]): React.FC<Props> }",
        "{ forwardRef(...a: any[]): string; forwardRef: typeof React.forwardRef }",
        "{ forwardRef: (...a: any[]) => object; forwardRef: any }",
        "{ get forwardRef(): typeof React.forwardRef }",
    ] {
        assert_eq!(with(literal), plain, "{literal}");
    }
    for literal in [
        "{ forwardRef: (...a: any[]) => string; forwardRef(...a: any[]): object }",
        "{ forwardRef(...a: any[]): object; forwardRef: (...a: any[]) => string }",
        "{ get forwardRef(): (...a: any[]) => object }",
    ] {
        assert_eq!(with(literal), Vec::<serde_json::Value>::new(), "{literal}");
    }
}

// BEGIN DIFFERENTIAL CASES. Copied verbatim into a scratch test on origin/main (oxc 0.158, which read every assertion
// as transparent) to print `BASE_DETECTIONS`; keep the two in step.

const DIFFERENTIAL_HEADER: &str = "import React, { forwardRef, memo } from 'react';\n\
                                   interface Props { label?: string }\n\
                                   interface ItemProps { size?: string }\n";

/// A detection form: its name, the callee it reads, and its source with `{C}` where the (wrapped) callee goes.
const DIFFERENTIAL_FORMS: &[(&str, &str, &str)] = &[
    ("memo", "memo", "export const Button = {C}((props: Props) => <div />);"),
    ("React.memo", "React.memo", "export const Button = {C}((props: Props) => <div />);"),
    ("forwardRef", "forwardRef", "export const Button = {C}<HTMLButtonElement, Props>((props, ref) => <div />);"),
    (
        "React.forwardRef, typed render",
        "React.forwardRef",
        "export const Button = {C}(function Button(props: Props, ref: any) { return <div />; });",
    ),
    (
        "wrapper call",
        "withTheme",
        "declare function withTheme<C>(c: C): C;\n\
         function Base(props: Props) { return <div />; }\n\
         export const Button = {C}(Base);",
    ),
    (
        "Object.assign",
        "Object.assign",
        "function Impl(props: Props) { return <div />; }\n\
         export const Root = {C}(Impl, { Item: (props: ItemProps) => <div /> });",
    ),
    ("PascalCase call returned", "Object", "export const Normalize = (props: Props) => {C}(props);"),
    ("element factory", "React.createElement", "export const Button = (props: Props) => {C}('div');"),
    (
        "factory",
        "createButton",
        "function createButton(): React.FC<Props> { return (props) => <div />; }\nexport const Button = {C}();",
    ),
    (
        "generic wrapper",
        "wrap",
        "function wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\n\
         export const Wrapped = {C}<Props>((props: Props) => <div />);",
    ),
    ("class", "React.Component", "export class Button extends {C}<Props> { render() { return <div />; } }"),
    (
        "call-only helper",
        "Helper",
        "function Helper(props: Props) { return <div />; }\n\
         export const Button = (props: Props) => <section>{{C}(props)}</section>;",
    ),
];

/// An assertion: its name, a line it needs declared first, and the wrapped callee, with `{b}` for the callee and
/// `{o}`/`{m}` for the namespace and member of a member callee (a row with them is only made for one).
const DIFFERENTIAL_ASSERTIONS: &[(&str, &str, &str)] = &[
    ("plain", "", "{b}"),
    ("parentheses", "", "({b})"),
    ("typeof itself", "", "({b} as typeof {b})"),
    ("typeof another", "declare const other: unknown;", "({b} as typeof other)"),
    ("alias of typeof itself", "type Alias = typeof {b};", "({b} as Alias)"),
    ("unresolved reference", "", "({b} as F)"),
    ("any", "", "({b} as any)"),
    ("unknown", "", "({b} as unknown)"),
    ("object", "", "({b} as object)"),
    ("callable literal, FC", "", "({b} as { (...args: any[]): React.FC<Props> })"),
    ("callable literal, object", "", "({b} as { (...args: any[]): object })"),
    ("callable literal, no result", "", "({b} as { (...args: any[]) })"),
    ("function, FC", "", "({b} as (...args: any[]) => React.FC<Props>)"),
    ("function, ReactElement", "", "({b} as (...args: any[]) => React.ReactElement)"),
    ("function, ReactNode", "", "({b} as (...args: any[]) => React.ReactNode)"),
    ("function, ReactElement | null", "", "({b} as (...args: any[]) => React.ReactElement | null)"),
    ("function, string", "", "({b} as (...args: any[]) => string)"),
    ("function, string | null", "", "({b} as (...args: any[]) => string | null)"),
    ("function, object", "", "({b} as (...args: any[]) => object)"),
    ("function, object literal", "", "({b} as (...args: any[]) => { id: string })"),
    ("function, void", "", "({b} as (...args: any[]) => void)"),
    ("function, unknown", "", "({b} as (...args: any[]) => unknown)"),
    ("function, unknown[]", "", "({b} as (...args: any[]) => unknown[])"),
    ("function, literal", "", "({b} as (...args: any[]) => 'x')"),
    ("generic function, FC", "", "({b} as <T>(...args: any[]) => React.FC<T>)"),
    ("object, then typeof itself", "", "({b} as object as typeof {b})"),
    ("typeof itself, then object", "", "({b} as typeof {b} as object)"),
    ("unknown, then function object", "", "({b} as unknown as (...args: any[]) => object)"),
    ("function object, then unknown", "", "({b} as (...args: any[]) => object as unknown)"),
    ("satisfies", "", "({b} satisfies typeof {b})"),
    ("non-null", "", "{b}!"),
    ("namespace any", "", "({o} as any).{m}"),
    ("namespace object", "", "({o} as object).{m}"),
    ("namespace typeof itself", "", "({o} as typeof {o}).{m}"),
    ("namespace member typeof itself", "", "({o} as { {m}: typeof {b} }).{m}"),
    ("namespace member function object", "", "({o} as { {m}: (...args: any[]) => object }).{m}"),
    ("namespace method object", "", "({o} as { {m}(...args: any[]): object }).{m}"),
    ("namespace method FC", "", "({o} as { {m}(...args: any[]): React.FC<Props> }).{m}"),
];

/// Every (form, assertion) row: its id `form | assertion` and its source.
fn differential_cases() -> Vec<(String, String)> {
    let mut cases = Vec::new();
    for (form, callee, template) in DIFFERENTIAL_FORMS {
        for (assertion, prelude, wrapped) in DIFFERENTIAL_ASSERTIONS {
            let member = callee.split_once('.');
            if member.is_none() && (wrapped.contains("{o}") || wrapped.contains("{m}")) {
                continue;
            }
            let (object, property) = member.unwrap_or((callee, callee));
            let fill = |text: &str| text.replace("{b}", callee).replace("{o}", object).replace("{m}", property);
            let source = format!("{DIFFERENTIAL_HEADER}{}\n{}", fill(prelude), template.replace("{C}", &fill(wrapped)));
            cases.push((format!("{form} | {assertion}"), source));
        }
    }
    cases
}

/// What a row extracts, as `name:props` pairs in order, or `parse error`.
fn differential_detections(source: &str) -> String {
    let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), source);
    if data.diagnostics.iter().any(|d| d.code == DiagnosticCode::ParseError) {
        return "parse error".to_owned();
    }
    let mut found: Vec<String> =
        data.component_mappings.iter().map(|m| format!("{}:{}", m.component_name, m.props_type_name)).collect();
    found.sort();
    found.join(" ")
}

// END DIFFERENTIAL CASES.

/// What the outermost assertion of a row is, as `component::Evidence` reads it: the rows whose expectation differs
/// from the base. `NotRenderable` declines the form's candidate; `NoComponent` declines it unless the call's result is
/// rendered rather than expected to be a component; `Unknown` keeps it but withholds the callee's declaration, which a
/// factory needs for any props.
fn differential_class(assertion: &str) -> &'static str {
    match assertion {
        "function, string" | "function, string | null" | "function, void" | "function, literal" => "NoComponent",
        "callable literal, object"
        | "function, object"
        | "function, object literal"
        | "function, unknown[]"
        | "unknown, then function object"
        | "namespace member function object"
        | "namespace method object" => "NotRenderable",
        "typeof another"
        | "unresolved reference"
        | "object"
        | "callable literal, FC"
        | "callable literal, no result"
        | "function, FC"
        | "function, ReactElement"
        | "function, ReactNode"
        | "function, ReactElement | null"
        | "function, unknown"
        | "generic function, FC"
        | "typeof itself, then object"
        | "namespace object"
        | "namespace method FC" => "Unknown",
        _ => "Identity",
    }
}

/// No recall regression: every row the base (origin/main, oxc 0.158) detected is detected the same, unless its
/// outermost assertion is affirmative evidence that the call is no component (`NotRenderable`, which drops the
/// form's own candidate), or a factory's callee lost the declaration its props come from (`Unknown`).
#[test]
fn every_form_the_base_detected_is_still_detected_unless_the_assertion_says_it_is_no_component() {
    let base: FxHashMap<&str, &str> = BASE_DETECTIONS.iter().copied().collect();
    let cases = differential_cases();
    assert_eq!(cases.len(), BASE_DETECTIONS.len(), "the base table and the cases are out of step");
    let mut failures = Vec::new();
    for (id, source) in &cases {
        let Some(&expected) = base.get(id.as_str()) else { panic!("{id} is not in BASE_DETECTIONS") };
        let (form, assertion) = id.split_once(" | ").unwrap();
        let subject = match form {
            "Object.assign" => "Root:",
            "PascalCase call returned" => "Normalize:",
            "generic wrapper" => "Wrapped:",
            // `Helper` is dropped as a helper and `Button` renders JSX of its own whatever the assertion says.
            "call-only helper" => "",
            _ => "Button:",
        };
        let actual = differential_detections(source);
        let found: Vec<&str> = actual.split(' ').collect();
        // A component invoked directly may give a string or a number: only a result React cannot render declines it.
        // An element factory asserted to give a string gives no element.
        let rendered = form == "PascalCase call returned";
        let declined = match differential_class(assertion) {
            "NotRenderable" => !subject.is_empty(),
            "NoComponent" => !subject.is_empty() && !rendered,
            "Unknown" => form == "factory",
            _ => false,
        };
        // Through a cast the base did not read `Object.assign` at all and published `Impl` under its own name; read
        // now, `Root` aliases it.
        let kept = |detection: &str| {
            found.contains(&detection) || (detection == "Impl:Props" && !declined && found.contains(&"Root:Props"))
        };
        let holds = actual != "parse error"
            && expected.split(' ').filter(|d| !d.is_empty() && !(declined && d.starts_with(subject))).all(kept)
            && !(declined && found.iter().any(|d| d.starts_with(subject)));
        if !holds {
            failures.push(format!("{id} (declined: {declined}): base `{expected}`, now `{actual}`\n{source}"));
        }
    }
    assert!(failures.is_empty(), "{} rows:\n\n{}", failures.len(), failures.join("\n\n"));
}

/// What origin/main (f4a88f93, oxc 0.158) extracts from each of `differential_cases()`, as `differential_detections`
/// prints it. Produced by copying the block between `BEGIN DIFFERENTIAL CASES` and `END DIFFERENTIAL CASES` into a
/// scratch test module on origin/main that prints each `(id, differential_detections(source))`; regenerate it the same
/// way when a case changes, never by hand.
const BASE_DETECTIONS: &[(&str, &str)] = &[
    ("memo | plain", "Button:Props"),
    ("memo | parentheses", "Button:Props"),
    ("memo | typeof itself", "Button:Props"),
    ("memo | typeof another", "Button:Props"),
    ("memo | alias of typeof itself", "Button:Props"),
    ("memo | unresolved reference", "Button:Props"),
    ("memo | any", "Button:Props"),
    ("memo | unknown", "Button:Props"),
    ("memo | object", "Button:Props"),
    ("memo | callable literal, FC", "Button:Props"),
    ("memo | callable literal, object", "Button:Props"),
    ("memo | callable literal, no result", "Button:Props"),
    ("memo | function, FC", "Button:Props"),
    ("memo | function, ReactElement", "Button:Props"),
    ("memo | function, ReactNode", "Button:Props"),
    ("memo | function, ReactElement | null", "Button:Props"),
    ("memo | function, string", "Button:Props"),
    ("memo | function, string | null", "Button:Props"),
    ("memo | function, object", "Button:Props"),
    ("memo | function, object literal", "Button:Props"),
    ("memo | function, void", "Button:Props"),
    ("memo | function, unknown", "Button:Props"),
    ("memo | function, unknown[]", "Button:Props"),
    ("memo | function, literal", "Button:Props"),
    ("memo | generic function, FC", "Button:Props"),
    ("memo | object, then typeof itself", "Button:Props"),
    ("memo | typeof itself, then object", "Button:Props"),
    ("memo | unknown, then function object", "Button:Props"),
    ("memo | function object, then unknown", "Button:Props"),
    ("memo | satisfies", "Button:Props"),
    ("memo | non-null", "Button:Props"),
    ("React.memo | plain", "Button:Props"),
    ("React.memo | parentheses", "Button:Props"),
    ("React.memo | typeof itself", "Button:Props"),
    ("React.memo | typeof another", "Button:Props"),
    ("React.memo | alias of typeof itself", "Button:Props"),
    ("React.memo | unresolved reference", "Button:Props"),
    ("React.memo | any", "Button:Props"),
    ("React.memo | unknown", "Button:Props"),
    ("React.memo | object", "Button:Props"),
    ("React.memo | callable literal, FC", "Button:Props"),
    ("React.memo | callable literal, object", "Button:Props"),
    ("React.memo | callable literal, no result", "Button:Props"),
    ("React.memo | function, FC", "Button:Props"),
    ("React.memo | function, ReactElement", "Button:Props"),
    ("React.memo | function, ReactNode", "Button:Props"),
    ("React.memo | function, ReactElement | null", "Button:Props"),
    ("React.memo | function, string", "Button:Props"),
    ("React.memo | function, string | null", "Button:Props"),
    ("React.memo | function, object", "Button:Props"),
    ("React.memo | function, object literal", "Button:Props"),
    ("React.memo | function, void", "Button:Props"),
    ("React.memo | function, unknown", "Button:Props"),
    ("React.memo | function, unknown[]", "Button:Props"),
    ("React.memo | function, literal", "Button:Props"),
    ("React.memo | generic function, FC", "Button:Props"),
    ("React.memo | object, then typeof itself", "Button:Props"),
    ("React.memo | typeof itself, then object", "Button:Props"),
    ("React.memo | unknown, then function object", "Button:Props"),
    ("React.memo | function object, then unknown", "Button:Props"),
    ("React.memo | satisfies", "Button:Props"),
    ("React.memo | non-null", "Button:Props"),
    ("React.memo | namespace any", "Button:Props"),
    ("React.memo | namespace object", "Button:Props"),
    ("React.memo | namespace typeof itself", "Button:Props"),
    ("React.memo | namespace member typeof itself", "Button:Props"),
    ("React.memo | namespace member function object", "Button:Props"),
    ("React.memo | namespace method object", "Button:Props"),
    ("React.memo | namespace method FC", "Button:Props"),
    ("forwardRef | plain", "Button:Props"),
    ("forwardRef | parentheses", "Button:Props"),
    ("forwardRef | typeof itself", "Button:Props"),
    ("forwardRef | typeof another", "Button:Props"),
    ("forwardRef | alias of typeof itself", "Button:Props"),
    ("forwardRef | unresolved reference", "Button:Props"),
    ("forwardRef | any", "Button:Props"),
    ("forwardRef | unknown", "Button:Props"),
    ("forwardRef | object", "Button:Props"),
    ("forwardRef | callable literal, FC", "Button:Props"),
    ("forwardRef | callable literal, object", "Button:Props"),
    ("forwardRef | callable literal, no result", "Button:Props"),
    ("forwardRef | function, FC", "Button:Props"),
    ("forwardRef | function, ReactElement", "Button:Props"),
    ("forwardRef | function, ReactNode", "Button:Props"),
    ("forwardRef | function, ReactElement | null", "Button:Props"),
    ("forwardRef | function, string", "Button:Props"),
    ("forwardRef | function, string | null", "Button:Props"),
    ("forwardRef | function, object", "Button:Props"),
    ("forwardRef | function, object literal", "Button:Props"),
    ("forwardRef | function, void", "Button:Props"),
    ("forwardRef | function, unknown", "Button:Props"),
    ("forwardRef | function, unknown[]", "Button:Props"),
    ("forwardRef | function, literal", "Button:Props"),
    ("forwardRef | generic function, FC", "Button:Props"),
    ("forwardRef | object, then typeof itself", "Button:Props"),
    ("forwardRef | typeof itself, then object", "Button:Props"),
    ("forwardRef | unknown, then function object", "Button:Props"),
    ("forwardRef | function object, then unknown", "Button:Props"),
    ("forwardRef | satisfies", "Button:Props"),
    ("forwardRef | non-null", "Button:Props"),
    ("React.forwardRef, typed render | plain", "Button:Props"),
    ("React.forwardRef, typed render | parentheses", "Button:Props"),
    ("React.forwardRef, typed render | typeof itself", "Button:Props"),
    ("React.forwardRef, typed render | typeof another", "Button:Props"),
    ("React.forwardRef, typed render | alias of typeof itself", "Button:Props"),
    ("React.forwardRef, typed render | unresolved reference", "Button:Props"),
    ("React.forwardRef, typed render | any", "Button:Props"),
    ("React.forwardRef, typed render | unknown", "Button:Props"),
    ("React.forwardRef, typed render | object", "Button:Props"),
    ("React.forwardRef, typed render | callable literal, FC", "Button:Props"),
    ("React.forwardRef, typed render | callable literal, object", "Button:Props"),
    ("React.forwardRef, typed render | callable literal, no result", "Button:Props"),
    ("React.forwardRef, typed render | function, FC", "Button:Props"),
    ("React.forwardRef, typed render | function, ReactElement", "Button:Props"),
    ("React.forwardRef, typed render | function, ReactNode", "Button:Props"),
    ("React.forwardRef, typed render | function, ReactElement | null", "Button:Props"),
    ("React.forwardRef, typed render | function, string", "Button:Props"),
    ("React.forwardRef, typed render | function, string | null", "Button:Props"),
    ("React.forwardRef, typed render | function, object", "Button:Props"),
    ("React.forwardRef, typed render | function, object literal", "Button:Props"),
    ("React.forwardRef, typed render | function, void", "Button:Props"),
    ("React.forwardRef, typed render | function, unknown", "Button:Props"),
    ("React.forwardRef, typed render | function, unknown[]", "Button:Props"),
    ("React.forwardRef, typed render | function, literal", "Button:Props"),
    ("React.forwardRef, typed render | generic function, FC", "Button:Props"),
    ("React.forwardRef, typed render | object, then typeof itself", "Button:Props"),
    ("React.forwardRef, typed render | typeof itself, then object", "Button:Props"),
    ("React.forwardRef, typed render | unknown, then function object", "Button:Props"),
    ("React.forwardRef, typed render | function object, then unknown", "Button:Props"),
    ("React.forwardRef, typed render | satisfies", "Button:Props"),
    ("React.forwardRef, typed render | non-null", "Button:Props"),
    ("React.forwardRef, typed render | namespace any", "Button:Props"),
    ("React.forwardRef, typed render | namespace object", "Button:Props"),
    ("React.forwardRef, typed render | namespace typeof itself", "Button:Props"),
    ("React.forwardRef, typed render | namespace member typeof itself", "Button:Props"),
    ("React.forwardRef, typed render | namespace member function object", "Button:Props"),
    ("React.forwardRef, typed render | namespace method object", "Button:Props"),
    ("React.forwardRef, typed render | namespace method FC", "Button:Props"),
    ("wrapper call | plain", "Button:Props"),
    ("wrapper call | parentheses", "Button:Props"),
    ("wrapper call | typeof itself", "Button:Props"),
    ("wrapper call | typeof another", "Button:Props"),
    ("wrapper call | alias of typeof itself", "Button:Props"),
    ("wrapper call | unresolved reference", "Button:Props"),
    ("wrapper call | any", "Button:Props"),
    ("wrapper call | unknown", "Button:Props"),
    ("wrapper call | object", "Button:Props"),
    ("wrapper call | callable literal, FC", "Button:Props"),
    ("wrapper call | callable literal, object", "Button:Props"),
    ("wrapper call | callable literal, no result", "Button:Props"),
    ("wrapper call | function, FC", "Button:Props"),
    ("wrapper call | function, ReactElement", "Button:Props"),
    ("wrapper call | function, ReactNode", "Button:Props"),
    ("wrapper call | function, ReactElement | null", "Button:Props"),
    ("wrapper call | function, string", "Button:Props"),
    ("wrapper call | function, string | null", "Button:Props"),
    ("wrapper call | function, object", "Button:Props"),
    ("wrapper call | function, object literal", "Button:Props"),
    ("wrapper call | function, void", "Button:Props"),
    ("wrapper call | function, unknown", "Button:Props"),
    ("wrapper call | function, unknown[]", "Button:Props"),
    ("wrapper call | function, literal", "Button:Props"),
    ("wrapper call | generic function, FC", "Button:Props"),
    ("wrapper call | object, then typeof itself", "Button:Props"),
    ("wrapper call | typeof itself, then object", "Button:Props"),
    ("wrapper call | unknown, then function object", "Button:Props"),
    ("wrapper call | function object, then unknown", "Button:Props"),
    ("wrapper call | satisfies", "Button:Props"),
    ("wrapper call | non-null", "Button:Props"),
    ("Object.assign | plain", "Item:ItemProps Root:Props"),
    ("Object.assign | parentheses", "Item:ItemProps Root:Props"),
    ("Object.assign | typeof itself", "Impl:Props"),
    ("Object.assign | typeof another", "Impl:Props"),
    ("Object.assign | alias of typeof itself", "Impl:Props"),
    ("Object.assign | unresolved reference", "Impl:Props"),
    ("Object.assign | any", "Impl:Props"),
    ("Object.assign | unknown", "Impl:Props"),
    ("Object.assign | object", "Impl:Props"),
    ("Object.assign | callable literal, FC", "Impl:Props"),
    ("Object.assign | callable literal, object", "Impl:Props"),
    ("Object.assign | callable literal, no result", "Impl:Props"),
    ("Object.assign | function, FC", "Impl:Props"),
    ("Object.assign | function, ReactElement", "Impl:Props"),
    ("Object.assign | function, ReactNode", "Impl:Props"),
    ("Object.assign | function, ReactElement | null", "Impl:Props"),
    ("Object.assign | function, string", "Impl:Props"),
    ("Object.assign | function, string | null", "Impl:Props"),
    ("Object.assign | function, object", "Impl:Props"),
    ("Object.assign | function, object literal", "Impl:Props"),
    ("Object.assign | function, void", "Impl:Props"),
    ("Object.assign | function, unknown", "Impl:Props"),
    ("Object.assign | function, unknown[]", "Impl:Props"),
    ("Object.assign | function, literal", "Impl:Props"),
    ("Object.assign | generic function, FC", "Impl:Props"),
    ("Object.assign | object, then typeof itself", "Impl:Props"),
    ("Object.assign | typeof itself, then object", "Impl:Props"),
    ("Object.assign | unknown, then function object", "Impl:Props"),
    ("Object.assign | function object, then unknown", "Impl:Props"),
    ("Object.assign | satisfies", "Impl:Props"),
    ("Object.assign | non-null", "Impl:Props"),
    ("Object.assign | namespace any", "Impl:Props"),
    ("Object.assign | namespace object", "Impl:Props"),
    ("Object.assign | namespace typeof itself", "Impl:Props"),
    ("Object.assign | namespace member typeof itself", "Impl:Props"),
    ("Object.assign | namespace member function object", "Impl:Props"),
    ("Object.assign | namespace method object", "Impl:Props"),
    ("Object.assign | namespace method FC", "Impl:Props"),
    ("PascalCase call returned | plain", "Normalize:Props"),
    ("PascalCase call returned | parentheses", "Normalize:Props"),
    ("PascalCase call returned | typeof itself", ""),
    ("PascalCase call returned | typeof another", ""),
    ("PascalCase call returned | alias of typeof itself", ""),
    ("PascalCase call returned | unresolved reference", ""),
    ("PascalCase call returned | any", ""),
    ("PascalCase call returned | unknown", ""),
    ("PascalCase call returned | object", ""),
    ("PascalCase call returned | callable literal, FC", ""),
    ("PascalCase call returned | callable literal, object", ""),
    ("PascalCase call returned | callable literal, no result", ""),
    ("PascalCase call returned | function, FC", ""),
    ("PascalCase call returned | function, ReactElement", ""),
    ("PascalCase call returned | function, ReactNode", ""),
    ("PascalCase call returned | function, ReactElement | null", ""),
    ("PascalCase call returned | function, string", ""),
    ("PascalCase call returned | function, string | null", ""),
    ("PascalCase call returned | function, object", ""),
    ("PascalCase call returned | function, object literal", ""),
    ("PascalCase call returned | function, void", ""),
    ("PascalCase call returned | function, unknown", ""),
    ("PascalCase call returned | function, unknown[]", ""),
    ("PascalCase call returned | function, literal", ""),
    ("PascalCase call returned | generic function, FC", ""),
    ("PascalCase call returned | object, then typeof itself", ""),
    ("PascalCase call returned | typeof itself, then object", ""),
    ("PascalCase call returned | unknown, then function object", ""),
    ("PascalCase call returned | function object, then unknown", ""),
    ("PascalCase call returned | satisfies", ""),
    ("PascalCase call returned | non-null", ""),
    ("element factory | plain", "Button:Props"),
    ("element factory | parentheses", "Button:Props"),
    ("element factory | typeof itself", ""),
    ("element factory | typeof another", ""),
    ("element factory | alias of typeof itself", ""),
    ("element factory | unresolved reference", ""),
    ("element factory | any", ""),
    ("element factory | unknown", ""),
    ("element factory | object", ""),
    ("element factory | callable literal, FC", ""),
    ("element factory | callable literal, object", ""),
    ("element factory | callable literal, no result", ""),
    ("element factory | function, FC", ""),
    ("element factory | function, ReactElement", ""),
    ("element factory | function, ReactNode", ""),
    ("element factory | function, ReactElement | null", ""),
    ("element factory | function, string", ""),
    ("element factory | function, string | null", ""),
    ("element factory | function, object", ""),
    ("element factory | function, object literal", ""),
    ("element factory | function, void", ""),
    ("element factory | function, unknown", ""),
    ("element factory | function, unknown[]", ""),
    ("element factory | function, literal", ""),
    ("element factory | generic function, FC", ""),
    ("element factory | object, then typeof itself", ""),
    ("element factory | typeof itself, then object", ""),
    ("element factory | unknown, then function object", ""),
    ("element factory | function object, then unknown", ""),
    ("element factory | satisfies", ""),
    ("element factory | non-null", ""),
    ("element factory | namespace any", "Button:Props"),
    ("element factory | namespace object", "Button:Props"),
    ("element factory | namespace typeof itself", "Button:Props"),
    ("element factory | namespace member typeof itself", "Button:Props"),
    ("element factory | namespace member function object", "Button:Props"),
    ("element factory | namespace method object", "Button:Props"),
    ("element factory | namespace method FC", "Button:Props"),
    ("factory | plain", "Button:createButton"),
    ("factory | parentheses", "Button:createButton"),
    ("factory | typeof itself", "Button:createButton"),
    ("factory | typeof another", "Button:createButton"),
    ("factory | alias of typeof itself", "Button:createButton"),
    ("factory | unresolved reference", "Button:createButton"),
    ("factory | any", "Button:createButton"),
    ("factory | unknown", "Button:createButton"),
    ("factory | object", "Button:createButton"),
    ("factory | callable literal, FC", "Button:createButton"),
    ("factory | callable literal, object", "Button:createButton"),
    ("factory | callable literal, no result", "Button:createButton"),
    ("factory | function, FC", "Button:createButton"),
    ("factory | function, ReactElement", "Button:createButton"),
    ("factory | function, ReactNode", "Button:createButton"),
    ("factory | function, ReactElement | null", "Button:createButton"),
    ("factory | function, string", "Button:createButton"),
    ("factory | function, string | null", "Button:createButton"),
    ("factory | function, object", "Button:createButton"),
    ("factory | function, object literal", "Button:createButton"),
    ("factory | function, void", "Button:createButton"),
    ("factory | function, unknown", "Button:createButton"),
    ("factory | function, unknown[]", "Button:createButton"),
    ("factory | function, literal", "Button:createButton"),
    ("factory | generic function, FC", "Button:createButton"),
    ("factory | object, then typeof itself", "Button:createButton"),
    ("factory | typeof itself, then object", "Button:createButton"),
    ("factory | unknown, then function object", "Button:createButton"),
    ("factory | function object, then unknown", "Button:createButton"),
    ("factory | satisfies", "Button:createButton"),
    ("factory | non-null", "Button:createButton"),
    ("generic wrapper | plain", "Wrapped:Props"),
    ("generic wrapper | parentheses", "Wrapped:Props"),
    ("generic wrapper | typeof itself", "Wrapped:Props"),
    ("generic wrapper | typeof another", "Wrapped:Props"),
    ("generic wrapper | alias of typeof itself", "Wrapped:Props"),
    ("generic wrapper | unresolved reference", "Wrapped:Props"),
    ("generic wrapper | any", "Wrapped:Props"),
    ("generic wrapper | unknown", "Wrapped:Props"),
    ("generic wrapper | object", "Wrapped:Props"),
    ("generic wrapper | callable literal, FC", "Wrapped:Props"),
    ("generic wrapper | callable literal, object", "Wrapped:Props"),
    ("generic wrapper | callable literal, no result", "Wrapped:Props"),
    ("generic wrapper | function, FC", "Wrapped:Props"),
    ("generic wrapper | function, ReactElement", "Wrapped:Props"),
    ("generic wrapper | function, ReactNode", "Wrapped:Props"),
    ("generic wrapper | function, ReactElement | null", "Wrapped:Props"),
    ("generic wrapper | function, string", "Wrapped:Props"),
    ("generic wrapper | function, string | null", "Wrapped:Props"),
    ("generic wrapper | function, object", "Wrapped:Props"),
    ("generic wrapper | function, object literal", "Wrapped:Props"),
    ("generic wrapper | function, void", "Wrapped:Props"),
    ("generic wrapper | function, unknown", "Wrapped:Props"),
    ("generic wrapper | function, unknown[]", "Wrapped:Props"),
    ("generic wrapper | function, literal", "Wrapped:Props"),
    ("generic wrapper | generic function, FC", "Wrapped:Props"),
    ("generic wrapper | object, then typeof itself", "Wrapped:Props"),
    ("generic wrapper | typeof itself, then object", "Wrapped:Props"),
    ("generic wrapper | unknown, then function object", "Wrapped:Props"),
    ("generic wrapper | function object, then unknown", "Wrapped:Props"),
    ("generic wrapper | satisfies", "Wrapped:Props"),
    ("generic wrapper | non-null", "Wrapped:Props"),
    ("class | plain", "Button:Props"),
    ("class | parentheses", "Button:Props"),
    ("class | typeof itself", ""),
    ("class | typeof another", ""),
    ("class | alias of typeof itself", ""),
    ("class | unresolved reference", ""),
    ("class | any", ""),
    ("class | unknown", ""),
    ("class | object", ""),
    ("class | callable literal, FC", ""),
    ("class | callable literal, object", ""),
    ("class | callable literal, no result", ""),
    ("class | function, FC", ""),
    ("class | function, ReactElement", ""),
    ("class | function, ReactNode", ""),
    ("class | function, ReactElement | null", ""),
    ("class | function, string", ""),
    ("class | function, string | null", ""),
    ("class | function, object", ""),
    ("class | function, object literal", ""),
    ("class | function, void", ""),
    ("class | function, unknown", ""),
    ("class | function, unknown[]", ""),
    ("class | function, literal", ""),
    ("class | generic function, FC", ""),
    ("class | object, then typeof itself", ""),
    ("class | typeof itself, then object", ""),
    ("class | unknown, then function object", ""),
    ("class | function object, then unknown", ""),
    ("class | satisfies", ""),
    ("class | non-null", ""),
    ("class | namespace any", ""),
    ("class | namespace object", ""),
    ("class | namespace typeof itself", ""),
    ("class | namespace member typeof itself", ""),
    ("class | namespace member function object", ""),
    ("class | namespace method object", ""),
    ("class | namespace method FC", ""),
    ("call-only helper | plain", "Button:Props"),
    ("call-only helper | parentheses", "Button:Props"),
    ("call-only helper | typeof itself", "Button:Props Helper:Props"),
    ("call-only helper | typeof another", "Button:Props"),
    ("call-only helper | alias of typeof itself", "Button:Props"),
    ("call-only helper | unresolved reference", "Button:Props"),
    ("call-only helper | any", "Button:Props"),
    ("call-only helper | unknown", "Button:Props"),
    ("call-only helper | object", "Button:Props"),
    ("call-only helper | callable literal, FC", "Button:Props"),
    ("call-only helper | callable literal, object", "Button:Props"),
    ("call-only helper | callable literal, no result", "Button:Props"),
    ("call-only helper | function, FC", "Button:Props"),
    ("call-only helper | function, ReactElement", "Button:Props"),
    ("call-only helper | function, ReactNode", "Button:Props"),
    ("call-only helper | function, ReactElement | null", "Button:Props"),
    ("call-only helper | function, string", "Button:Props"),
    ("call-only helper | function, string | null", "Button:Props"),
    ("call-only helper | function, object", "Button:Props"),
    ("call-only helper | function, object literal", "Button:Props"),
    ("call-only helper | function, void", "Button:Props"),
    ("call-only helper | function, unknown", "Button:Props"),
    ("call-only helper | function, unknown[]", "Button:Props"),
    ("call-only helper | function, literal", "Button:Props"),
    ("call-only helper | generic function, FC", "Button:Props"),
    ("call-only helper | object, then typeof itself", "Button:Props Helper:Props"),
    ("call-only helper | typeof itself, then object", "Button:Props Helper:Props"),
    ("call-only helper | unknown, then function object", "Button:Props"),
    ("call-only helper | function object, then unknown", "Button:Props"),
    ("call-only helper | satisfies", "Button:Props Helper:Props"),
    ("call-only helper | non-null", "Button:Props"),
];

// Review round 6.

const LIST: &str = "interface ListProps<T> { items?: T[] }\n\
                    interface ItemProps { size?: string }\n\
                    interface Row { id: string }\n\
                    declare function withTheme<C>(c: C): C;\n\
                    function List<T>(props: ListProps<T>) { return <div />; }\n";

/// The mappings of `source` after `HEADER`, by name, and the messages of its `GenericArgumentMismatch` diagnostics.
fn with_mismatches(source: &str) -> (Vec<serde_json::Value>, Vec<String>) {
    let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), &format!("{HEADER}{source}"));
    let mismatches = data
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::GenericArgumentMismatch)
        .map(|diagnostic| format!("{:?}: {}", diagnostic.severity, diagnostic.message))
        .collect();
    let mut mappings = tsx(source);
    mappings.sort_by(|a, b| a["component_name"].as_str().cmp(&b["component_name"].as_str()));
    (mappings, mismatches)
}

/// Finding 1: `List<Row>` binds `List`'s type parameter at every site that makes a component of it, so the props are
/// `ListProps<Row>`; a form whose arguments cannot be bound keeps `ListProps<T>` and says so once.
#[test]
fn an_instantiated_component_binds_its_type_parameters_or_says_it_did_not() {
    let sites = [
        "export const RowList = {V};",
        "export const RowList = memo({V});",
        "export const RowList = withTheme({V});",
        "export const RowList = Object.assign({V}, { Item: (props: ItemProps) => <div /> });",
    ];
    let row = serde_json::json!([{ "n": "Row", "a": [] }]);
    let mut failures = Vec::new();
    for site in sites {
        let (plain, none) = with_mismatches(&format!("{LIST}{}", site.replace("{V}", "List")));
        let list = plain.iter().find(|m| m["component_name"] == "RowList").unwrap().clone();
        assert_eq!(list["props_type_name"], "ListProps", "{list:#}");
        assert_eq!(list["props_type_args"], serde_json::json!([{ "n": "T", "a": [] }]), "{list:#}");
        assert_eq!(list["type_params"], serde_json::json!(["T"]), "{list:#}");
        assert_eq!(none, Vec::<String>::new());
        let mut bound = plain.clone();
        for mapping in &mut bound {
            if mapping["component_name"] == "RowList" {
                mapping["props_type_args"] = row.clone();
                mapping["type_params"] = serde_json::json!([]);
            }
        }
        for value in ["List<Row>", "(List<Row>)", "(List<Row>)!", "(List<Row>) satisfies unknown", "(List<Row>) as any"]
        {
            let actual = with_mismatches(&format!("{LIST}{}", site.replace("{V}", value)));
            if actual != (bound.clone(), Vec::new()) {
                failures.push(format!("{site} with {value}: {actual:#?}"));
            }
        }
        for value in ["((List as any)<Row>)!", "List<Row, Row>"] {
            let (mappings, mismatches) = with_mismatches(&format!("{LIST}{}", site.replace("{V}", value)));
            if mappings != plain || mismatches.len() != 1 || !mismatches[0].starts_with("Warning: 'RowList' gives") {
                failures.push(format!("{site} with {value}: {mappings:#?} {mismatches:?}"));
            }
        }
    }
    // A type parameter used inside a props type argument is not bound.
    let grid = "interface GridProps<T> { rows?: T }\nfunction Grid<T>(props: GridProps<T[]>) { return <div />; }\n";
    let (plain, _) = with_mismatches(&format!("{grid}export const RowGrid = Grid;"));
    let (mappings, mismatches) = with_mismatches(&format!("{grid}export const RowGrid = Grid<string>;"));
    assert_eq!(mappings, plain);
    assert_eq!(
        mismatches,
        ["Warning: 'RowGrid' gives component 'Grid' type arguments that are not applied to its props (a type \
          parameter is used inside a props type argument): the props keep 'Grid''s type parameters"]
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Finding 1, the imported sibling: `ComponentProps<typeof Imported>` cannot carry type arguments, so it says so.
#[test]
fn an_instantiated_imported_component_says_its_type_arguments_are_not_applied() {
    let import = "import { List } from './list';\n";
    let (plain, none) = with_mismatches(&format!("{import}export const RowList = List;"));
    assert_eq!(none, Vec::<String>::new());
    let (mappings, mismatches) = with_mismatches(&format!("{import}export const RowList = List<string>;"));
    assert_eq!(mappings, plain);
    assert_eq!(
        mismatches,
        ["Warning: 'RowList' gives imported component 'List' type arguments that are not applied to its props: the \
          props keep 'List''s type parameters"]
    );
}

/// Finding 3: a name declared again in an inner scope shadows the module's `typeof` alias only there; the module's
/// own second declaration of it makes it ambiguous everywhere.
#[test]
fn a_typeof_alias_is_shadowed_only_where_its_name_is_declared_again() {
    let factory = "function createButton(): React.FC<Props> { return (props) => <div />; }\n\
                   type Factory = typeof createButton;\n";
    let plain = tsx(&format!("{factory}export const Button = createButton();"));
    assert_eq!(plain.len(), 1, "{plain:#?}");
    let asserted = |other: &str| tsx(&format!("{factory}{other}\nexport const Button = (createButton as Factory)();"));
    for other in [
        "const Factory = 1;",
        "function g<Factory>(f: Factory) { return f; }",
        "const h = <Factory,>(f: Factory) => f;",
        "class K<Factory> { f?: Factory }",
        "type Mapped = { [Factory in 'a']: Factory };",
        "type G<Factory> = Factory;",
        "type I<T> = T extends Array<infer Factory> ? Factory : never;",
        "namespace N { export type Factory = () => string; }",
        "namespace N.M { interface Factory { (): string } }",
        "declare module 'm' { type Factory = () => string; }",
        "function f() { type Factory = () => string; }",
    ] {
        assert_eq!(asserted(other), plain, "{other}");
    }
    for other in [
        "import type { Factory } from './factory';",
        "import { Factory as Factory } from './factory';",
        "import * as Factory from './factory';",
        "interface Factory { (): string }",
        "class Factory {}",
        "export enum Factory { A }",
        "namespace Factory {}",
    ] {
        assert_eq!(asserted(other), Vec::<serde_json::Value>::new(), "{other}");
        // The declined factory call is the skip any unmatched PascalCase binding is, not silence.
        let source = format!("{HEADER}{factory}{other}\nexport const Button = (createButton as Factory)();");
        let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), &source);
        let skipped: Vec<&str> = data
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::SkippedCandidate && d.message.starts_with("'Button'"))
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(
            skipped,
            [
                "'Button' is a PascalCase binding but matched no known component pattern (FC annotation, forwardRef, HOC \
              wrapper, or identifier alias)"
            ],
            "{other}"
        );
    }
}

/// Finding 4: a function that never returns is assignable to every function type, so a `never` result is no evidence:
/// the named reader, the rendered reader and a namespace member all keep the call.
#[test]
fn a_never_result_is_no_evidence_against_a_call() {
    let plain = tsx("export const Button = memo((props: Props) => <div />);");
    assert_eq!(plain.len(), 1, "{plain:#?}");
    for callee in
        ["(memo as (c: any) => never)", "(memo as { (c: any): never })", "(memo as (c: any) => string | never)"]
    {
        let source = format!("export const Button = {callee}((props: Props) => <div />);");
        let expected = if callee.contains("string") { Vec::new() } else { plain.clone() };
        assert_eq!(tsx(&source), expected, "{callee}");
    }
    let render = "<HTMLButtonElement, Props>((props, ref) => <div />);";
    let forward = tsx(&format!("export const Button = React.forwardRef{render}"));
    let member =
        tsx(&format!("export const Button = (React as {{ forwardRef: (...a: any[]) => never }}).forwardRef{render}"));
    assert_eq!(member, forward);
    let rendered = |callee: &str| {
        tsx(&format!(
            "function Greeting(props: Props) {{ return <div />; }}\n\
             export const Button = (props: Props) => {callee}(props);\nexport {{ Greeting }};"
        ))
        .iter()
        .map(|m| m["component_name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>()
    };
    assert_eq!(rendered("Greeting"), ["Greeting", "Button"]);
    assert_eq!(rendered("(Greeting as (p: Props) => never)"), ["Greeting", "Button"]);
    assert_eq!(rendered("(Greeting as (p: Props) => object)"), ["Greeting"]);
}

/// Finding 5: a setter is never what a read of the member calls, so it answers nothing; an untyped declaration that
/// could answer the call keeps it unknown.
#[test]
fn a_setter_answers_nothing_and_an_untyped_declaration_stays_unknown() {
    let render = "<HTMLButtonElement, Props>((props, ref) => <div />);";
    let plain = tsx(&format!("export const Button = React.forwardRef{render}"));
    assert_eq!(plain.len(), 1, "{plain:#?}");
    let with = |literal: &str| tsx(&format!("export const Button = (React as {literal}).forwardRef{render}"));
    for literal in [
        "{ get forwardRef(): (...a: any[]) => object; set forwardRef(v: (...a: any[]) => object) }",
        "{ set forwardRef(v: unknown); get forwardRef(): (...a: any[]) => object }",
        "{ forwardRef(...a: any[]): object; set forwardRef(v: (...a: any[]) => object) }",
    ] {
        assert_eq!(with(literal), Vec::<serde_json::Value>::new(), "{literal}");
    }
    for literal in [
        "{ forwardRef(...a: any[]): object; forwardRef }",
        "{ forwardRef(...a: any[]): object; forwardRef(...a: any[]) }",
        "{ get forwardRef(); set forwardRef(v: unknown) }",
        "{ set forwardRef(v: (...a: any[]) => object) }",
    ] {
        assert_eq!(with(literal), plain, "{literal}");
    }
}

/// Finding 6, as the output shows it: an element factory asserted to give a string renders nothing, so a ref-taking
/// function passed only to it is a render callback, as when nothing renders it.
#[test]
fn an_element_factory_asserted_to_a_string_renders_nothing() {
    let names = |call: &str| {
        tsx(&format!(
            "import {{ createElement }} from 'react';\n\
             function Foo(props: Props, ref: unknown) {{ return <div />; }}\n\
             export const el = {call}(Foo);"
        ))
        .iter()
        .map(|m| m["component_name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>()
    };
    assert_eq!(names("createElement"), ["Foo"]);
    assert_eq!(names("(createElement as typeof createElement)"), ["Foo"]);
    assert_eq!(names("(createElement as (t: any) => React.ReactElement)"), ["Foo"]);
    assert_eq!(names("(createElement as (t: any) => string)"), Vec::<String>::new());
    assert_eq!(names("(createElement as (t: any) => object)"), Vec::<String>::new());
    assert_eq!(names("String"), Vec::<String>::new());
}

/// Finding 6's sibling: a function returning an element factory's result asserted to a string returns no element.
#[test]
fn a_function_returning_a_factory_result_asserted_to_a_string_returns_no_element() {
    let names = |call: &str| {
        tsx(&format!("import {{ createElement }} from 'react';\nexport const Button = (props: Props) => {call};"))
            .iter()
            .map(|m| m["component_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(names("createElement('div')"), ["Button"]);
    assert_eq!(names("(createElement as (t: any) => React.ReactElement)('div')"), ["Button"]);
    assert_eq!(names("(createElement as (t: any) => string)('div')"), Vec::<String>::new());
    assert_eq!(names("([1].map as (f: unknown) => string)(() => <div />)"), Vec::<String>::new());
    assert_eq!(names("([1].map as (f: unknown) => unknown)(() => <div />)"), ["Button"]);
}

/// Finding 7, dismissed: calling a hook is the evidence that a function is a component, whatever an assertion says the
/// hook returns, `never` included.
#[test]
fn a_hook_called_through_a_never_assertion_is_still_a_hook_call() {
    let names = |hook: &str| {
        let source = format!(
            "declare function useThing(): object;\n\
             export const Panel = (props: Props) => {{ {hook}(); return props.label; }};"
        );
        tsx(&source).iter().map(|m| m["component_name"].as_str().unwrap().to_owned()).collect::<Vec<_>>()
    };
    assert_eq!(names("(useThing as () => never)"), ["Panel"]);
    assert_eq!(names("(useThing as () => string)"), ["Panel"]);
}

/// Finding 2: a module without a `typeof` alias is not walked for the names that could shadow one; a module with one is
/// walked once.
#[test]
fn only_a_module_with_a_typeof_alias_is_walked_for_shadowing_names() {
    let walks = |source: &str| {
        super::component::steps::take_walks();
        parse_file(Utf8Path::new("/fixtures/Button.tsx"), &format!("{HEADER}{source}"));
        super::component::steps::take_walks()
    };
    let generic = "function g<Factory>(f: Factory) { return f; }\ntype Mapped = { [K in 'a']: K };\n\
                   export const Button = memo((props: Props) => <div />);";
    assert_eq!(walks(generic), 0);
    assert_eq!(walks(&format!("type Size = typeof size;\ndeclare const size: number;\n{generic}")), 1);
}

/// Finding 3: a type parameter or a declaration of the alias's name in a scope around the cast shadows the alias there,
/// and only there.
#[test]
fn a_redeclaration_around_the_cast_shadows_the_alias() {
    let means = |source: &str| {
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, source, oxc_span::SourceType::tsx()).parse();
        let aliases = super::component::typeof_aliases(&parsed.program);
        let at = u32::try_from(source.rfind("as Factory").unwrap() + 3).unwrap();
        aliases.get("Factory").map(|alias| alias.means_at(oxc_span::Span::new(at, at + 7)))
    };
    let alias = "declare function make(): unknown;\ntype Factory = typeof make;\n";
    for (scope, expected) in [
        ("const v = (make as Factory)();", Some(true)),
        ("function g<T>() { return (make as Factory)(); }", Some(true)),
        ("function g<Factory>() { return (make as Factory)(); }", Some(false)),
        ("function g<Factory>() {}\nconst v = (make as Factory)();", Some(true)),
        ("const g = <Factory,>() => (make as Factory)();", Some(false)),
        ("class K<Factory> { m() { return (make as Factory)(); } }", Some(false)),
        ("class K { m<Factory>() { return (make as Factory)(); } }", Some(false)),
        ("namespace N { type Factory = string; const v = (make as Factory)(); }", Some(false)),
        ("namespace N { type Factory = string; }\nconst v = (make as Factory)();", Some(true)),
        ("function f() { interface Factory {} return (make as Factory)(); }", Some(false)),
        ("function f() { { class Factory {} } return (make as Factory)(); }", Some(true)),
        ("const C = class Factory { m() { return (make as Factory)(); } };", Some(false)),
        ("type M = { [Factory in 'a']: Factory };\nconst v = (make as Factory)();", Some(true)),
        ("import type { Factory } from './f';\nconst v = (make as Factory)();", None),
    ] {
        assert_eq!(means(&format!("{alias}{scope}")), expected, "{scope}");
    }
}

/// Finding 6: the exact render counts of the element factory reader, through each assertion.
#[test]
fn an_element_factory_renders_its_argument_unless_asserted_to_give_no_element() {
    let rendered = |call: &str| {
        let source = format!("{HEADER}import {{ createElement }} from 'react';\nconst el = {call}(Foo);");
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &source, oxc_span::SourceType::tsx()).parse();
        let mut collector = SourceDataCollector::new(Utf8Path::new("/fixtures/Button.tsx"), &source, true);
        oxc_ast_visit::Visit::visit_program(&mut collector, &parsed.program);
        collector.name_uses.get("Foo").map_or(0, super::uses::NameUses::rendered)
    };
    assert_eq!(rendered("createElement"), 1);
    assert_eq!(rendered("React.createElement"), 1);
    assert_eq!(rendered("(createElement as typeof createElement)"), 1);
    assert_eq!(rendered("(createElement as any)"), 1);
    assert_eq!(rendered("(createElement as CreateElement)"), 1);
    assert_eq!(rendered("(createElement as (t: any) => unknown)"), 1);
    assert_eq!(rendered("(createElement as (t: any) => string)"), 0);
    assert_eq!(rendered("(createElement as (t: any) => number | boolean)"), 0);
    assert_eq!(rendered("(createElement as (t: any) => object)"), 0);
    assert_eq!(rendered("(React as { createElement: (t: any) => string }).createElement"), 0);
    // A component wrapper renders its argument by its name, whatever it returns.
    assert_eq!(rendered("memo"), 1);
}
