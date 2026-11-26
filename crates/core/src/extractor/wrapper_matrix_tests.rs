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

    /// What the wrapper gives a site that reads `signatures`: a function type returning `object` declines a call, but
    /// has no construct signature, so it says nothing about an `extends` target.
    fn expect(self, signatures: super::component::Signatures) -> Expect {
        match (self, signatures) {
            (
                Self::AsObjectResult | Self::AsUnknownAsObjectResult | Self::TypeAssertionObjectResult,
                super::component::Signatures::Construct,
            ) => return Expect::SameWithoutCallee,
            (_, super::component::Signatures::Call | super::component::Signatures::Construct) => {}
        }
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
                // The class forms wrap an `extends` target, which is constructed.
                let signatures = if form.name.contains("class") {
                    super::component::Signatures::Construct
                } else {
                    super::component::Signatures::Call
                };
                let expect = if source == plain { Expect::Same } else { callee.expect(signatures) };
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
        ("(Object as (value: object) => void)", false),
        ("(Object as (value: object) => (object))", false),
        ("(Object as (value: object) => { id: string })", true),
        ("(Object as (value: object) => unknown[])", true),
        ("(Object as (value: object) => object | null)", true),
        ("(Object as (value: object) => object & React.ReactElement)", true),
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
        // A string is a node: off the closed list, so no evidence.
        "(React as { forwardRef(...args: any[]): string })",
    ] {
        assert_eq!(tsx(&format!("export const Button = {receiver}.forwardRef{render}")), plain, "{receiver}");
    }
    for receiver in
        ["(React as { forwardRef: (...args: any[]) => object })", "(React as { forwardRef(...args: any[]): void })"]
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
        "{ (a: unknown): string; (b: unknown): void }",
    ] {
        assert_eq!(tsx(&format!("export const Button = (memo as {ty})((props: Props) => <div />);")), plain, "{ty}");
    }
    for ty in ["{ (render: React.FC<Props>): object }", "{ (a: unknown): symbol; (b: unknown): void }"] {
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
        "{ forwardRef: (...a: any[]) => string; forwardRef(...a: any[]): object }",
        "{ forwardRef(...a: any[]): object; forwardRef: (...a: any[]) => string }",
    ] {
        assert_eq!(with(literal), plain, "{literal}");
    }
    for literal in [
        "{ forwardRef: (...a: any[]) => void; forwardRef(...a: any[]): object }",
        "{ forwardRef(...a: any[]): object; forwardRef: (...a: any[]) => symbol }",
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
    (
        "wrapper naming the props",
        "ui.withContext",
        "declare const ui: { withContext<P>(View: React.ComponentType<P>): React.FC<P> };\n\
         function Base(props: ItemProps) { return <div />; }\n\
         export const Button = {C}<Props>(Base);",
    ),
    (
        "wrapper with two type arguments",
        "wrap2",
        "declare function wrap2<E, P>(View: React.ComponentType<P>): React.FC<P>;\n\
         function Base(props: ItemProps) { return <div />; }\n\
         export const Button = {C}<HTMLDivElement, Props>(Base);",
    ),
    // The value a wrapper or `Object.assign` receives, wrapped in place of a callee.
    (
        "Object.assign argument",
        "Impl",
        "function Impl(props: Props) { return <div />; }\n\
         export const Root = Object.assign({C}, { Item: (props: ItemProps) => <div /> });",
    ),
    ("memo argument", "Impl", "function Impl(props: Props) { return <div />; }\nexport const Button = memo({C});"),
    ("context", "React.createContext", "declare const Default: Props;\nexport const Button = {C}<Props>(Default);"),
    ("class", "React.Component", "export class Button extends {C}<Props> { render() { return <div />; } }"),
    (
        "call-only helper",
        "Helper",
        "function Helper(props: Props) { return <div />; }\n\
         export const Button = (props: Props) => <section>{{C}(props)}</section>;",
    ),
];

/// An assertion: its name, a line it needs declared first, the wrapped callee, with `{b}` for the callee and `{o}`/`{m}`
/// for the namespace and member of a member callee (a row with them is only made for one), whether it keeps the
/// callee's own type, and the entry of [`CLOSED_LIST`] the call's whole result is, if any. The last two are the oracle,
/// written from TypeScript's assignability, not read from the extractor.
const DIFFERENTIAL_ASSERTIONS: &[(&str, &str, &str, bool, Option<&str>)] = &[
    ("plain", "", "{b}", true, None),
    ("parentheses", "", "({b})", true, None),
    ("typeof itself", "", "({b} as typeof {b})", true, None),
    ("typeof another", "declare const other: unknown;", "({b} as typeof other)", false, None),
    ("alias of typeof itself", "type Alias = typeof {b};", "({b} as Alias)", true, None),
    ("unresolved reference", "", "({b} as F)", false, None),
    ("any", "", "({b} as any)", true, None),
    ("unknown", "", "({b} as unknown)", true, None),
    // Not callable at all: no call result to read.
    ("object", "", "({b} as object)", false, None),
    ("callable literal, FC", "", "({b} as { (...args: any[]): React.FC<Props> })", false, None),
    ("callable literal, object", "", "({b} as { (...args: any[]): object })", false, Some("object")),
    ("callable literal, no result", "", "({b} as { (...args: any[]) })", false, None),
    ("function, FC", "", "({b} as (...args: any[]) => React.FC<Props>)", false, None),
    ("function, ReactElement", "", "({b} as (...args: any[]) => React.ReactElement)", false, None),
    ("function, ReactNode", "", "({b} as (...args: any[]) => React.ReactNode)", false, None),
    ("function, ReactElement | null", "", "({b} as (...args: any[]) => React.ReactElement | null)", false, None),
    ("function, string", "", "({b} as (...args: any[]) => string)", false, None),
    ("function, string | null", "", "({b} as (...args: any[]) => string | null)", false, None),
    ("function, object", "", "({b} as (...args: any[]) => object)", false, Some("object")),
    ("function, object literal", "", "({b} as (...args: any[]) => { id: string })", false, None),
    ("function, void", "", "({b} as (...args: any[]) => void)", false, Some("void")),
    ("function, unknown", "", "({b} as (...args: any[]) => unknown)", false, None),
    ("function, unknown[]", "", "({b} as (...args: any[]) => unknown[])", false, None),
    ("function, literal", "", "({b} as (...args: any[]) => 'x')", false, None),
    ("generic function, FC", "", "({b} as <T>(...args: any[]) => React.FC<T>)", false, None),
    ("object, then typeof itself", "", "({b} as object as typeof {b})", true, None),
    ("typeof itself, then object", "", "({b} as typeof {b} as object)", false, None),
    ("unknown, then function object", "", "({b} as unknown as (...args: any[]) => object)", false, Some("object")),
    ("function object, then unknown", "", "({b} as (...args: any[]) => object as unknown)", true, None),
    ("satisfies", "", "({b} satisfies typeof {b})", true, None),
    ("non-null", "", "{b}!", true, None),
    ("namespace any", "", "({o} as any).{m}", true, None),
    ("namespace object", "", "({o} as object).{m}", false, None),
    ("namespace typeof itself", "", "({o} as typeof {o}).{m}", true, None),
    ("namespace member typeof itself", "", "({o} as { {m}: typeof {b} }).{m}", true, None),
    ("namespace member function object", "", "({o} as { {m}: (...args: any[]) => object }).{m}", false, Some("object")),
    ("namespace method object", "", "({o} as { {m}(...args: any[]): object }).{m}", false, Some("object")),
    ("namespace method FC", "", "({o} as { {m}(...args: any[]): React.FC<Props> }).{m}", false, None),
    // The namespace's own call signature is not the member's: only the member's type says what calling it gives.
    (
        "namespace call signature, member typeof itself",
        "",
        "({o} as unknown as { (): void; {m}: typeof {b} }).{m}",
        true,
        None,
    ),
    // Round 12: construct and call signatures side by side; `CONSTRUCT_CLOSED` gives the construct side's entries.
    (
        "construct Component, call void",
        "",
        "({b} as unknown as { new <P>(props: P): React.Component<P>; (...args: any[]): void })",
        false,
        Some("void"),
    ),
    (
        "construct object, call FC",
        "",
        "({b} as unknown as { new (...args: any[]): object; (...args: any[]): React.FC<Props> })",
        false,
        None,
    ),
    ("constructor type, object", "", "({b} as unknown as new (...args: any[]) => object)", false, None),
    ("constructor type, Component", "", "({b} as unknown as new <P>(props: P) => React.Component<P>)", false, None),
    (
        "namespace member construct Component, call void",
        "",
        "({o} as unknown as { {m}: { new <P>(props: P): React.Component<P>; (...args: any[]): void } }).{m}",
        false,
        Some("void"),
    ),
];

/// The assertions whose construct signatures all give a closed-list entry, for a site that constructs (`extends`).
const CONSTRUCT_CLOSED: &[(&str, &str)] =
    &[("construct object, call FC", "object"), ("constructor type, object", "object")];

/// The closed list as the oracle states it: the call results TypeScript cannot assign to `ReactNode | ComponentType |
/// ReactElement` under any instantiation. `bigint` is a member of `ReactNode` since @types/react 18.3.
const CLOSED_LIST: &[&str] = &["object", "symbol", "void"];

/// The return-type axis: each call result asserted on each form's callee, by name and as written.
const RESULT_TYPES: &[(&str, &str)] = &[
    ("any", "any"),
    ("any[]", "any[]"),
    ("unknown", "unknown"),
    ("unknown[]", "unknown[]"),
    ("object", "object"),
    ("symbol", "symbol"),
    ("void", "void"),
    ("never", "never"),
    ("string", "string"),
    ("number", "number"),
    ("null", "null"),
    ("boolean", "boolean"),
    ("ReactNode", "React.ReactNode"),
    ("ReactElement", "React.ReactElement"),
    ("JSX.Element", "JSX.Element"),
    ("FC<P>", "React.FC<Props>"),
    ("ComponentType<P>", "React.ComponentType<Props>"),
    ("element-shaped literal", "{ type: string; props: {}; key: string | null }"),
    ("empty literal", "{}"),
];

/// How the return-type axis asserts a result `{R}`: the shape's name, the wrapped callee, and whether `{R}` is the
/// call's whole result (a union or an intersection with it is another type).
const RESULT_SHAPES: &[(&str, &str, bool)] = &[
    ("function", "({b} as (...args: any[]) => {R})", true),
    ("callable literal", "({b} as { (...args: any[]): {R} })", true),
    ("union", "({b} as (...args: any[]) => {R} | null)", false),
    ("intersection", "({b} as (...args: any[]) => {R} & { id?: string })", false),
    ("namespace method", "({o} as { {m}(...args: any[]): {R} }).{m}", true),
];

/// Results the axis does not build from a `RESULT_TYPES` entry: a type parameter, an indexed access, a conditional.
const RESULT_EXTRAS: &[(&str, &str)] = &[
    ("returns generic T", "({b} as <T>(...args: any[]) => T)"),
    ("returns indexed access", "({b} as (...args: any[]) => Props['label'])"),
    ("returns conditional", "({b} as (...args: any[]) => Props extends object ? object : symbol)"),
];

/// One (form, assertion) row of the differential.
struct Case {
    /// `form | assertion`.
    id: String,
    source: String,
    /// The assertion keeps the callee's own type.
    keeps: bool,
    /// The entry of [`CLOSED_LIST`] the call's whole result is.
    closed: Option<&'static str>,
}

/// Every (form, assertion) row, the return-type axis included.
fn differential_cases() -> Vec<Case> {
    let mut assertions: Vec<(String, &str, String, bool, Option<&'static str>)> = DIFFERENTIAL_ASSERTIONS
        .iter()
        .map(|&(name, prelude, wrapped, keeps, closed)| (name.to_owned(), prelude, wrapped.to_owned(), keeps, closed))
        .collect();
    for &(result, ty) in RESULT_TYPES {
        for &(shape, template, whole) in RESULT_SHAPES {
            let closed = CLOSED_LIST.iter().copied().find(|&entry| whole && entry == result);
            assertions.push((format!("returns {result}, {shape}"), "", template.replace("{R}", ty), false, closed));
        }
    }
    for &(name, wrapped) in RESULT_EXTRAS {
        assertions.push((name.to_owned(), "", wrapped.to_owned(), false, None));
    }
    let mut cases = Vec::new();
    for (form, callee, template) in DIFFERENTIAL_FORMS {
        for (assertion, prelude, wrapped, keeps, closed) in &assertions {
            let member = callee.split_once('.');
            if member.is_none() && (wrapped.contains("{o}") || wrapped.contains("{m}")) {
                continue;
            }
            let (object, property) = member.unwrap_or((callee, callee));
            let fill = |text: &str| text.replace("{b}", callee).replace("{o}", object).replace("{m}", property);
            let source = format!("{DIFFERENTIAL_HEADER}{}\n{}", fill(prelude), template.replace("{C}", &fill(wrapped)));
            // The class form's target is constructed: its construct signatures' entry, which only `CONSTRUCT_CLOSED`
            // names (every other row's shape is a call signature).
            let closed = if *form == "class" {
                CONSTRUCT_CLOSED.iter().find(|(name, _)| name == assertion).map(|(_, entry)| *entry)
            } else {
                *closed
            };
            cases.push(Case { id: format!("{form} | {assertion}"), source, keeps: *keeps, closed });
        }
    }
    cases
}

/// What a row extracts, as `name:props` pairs in order, `-` for none, or `parse error`.
fn differential_detections(source: &str) -> String {
    let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), source);
    if data.diagnostics.iter().any(|d| d.code == DiagnosticCode::ParseError) {
        return "parse error".to_owned();
    }
    let mut found: Vec<String> =
        data.component_mappings.iter().map(|m| format!("{}:{}", m.component_name, m.props_type_name)).collect();
    found.sort();
    if found.is_empty() { "-".to_owned() } else { found.join(" ") }
}

// END DIFFERENTIAL CASES.

/// The component a form's candidate publishes, as a `name:` prefix: empty for `call-only helper`, where `Helper` is
/// dropped as a helper and `Button` renders JSX of its own whatever the assertion says.
fn differential_subject(form: &str) -> &'static str {
    match form {
        "Object.assign" | "Object.assign argument" => "Root:",
        "PascalCase call returned" => "Normalize:",
        "generic wrapper" => "Wrapped:",
        "call-only helper" => "",
        _ => "Button:",
    }
}

/// Whether a row declines its form's candidate, and why: its call's whole result is on the closed list, or the form
/// is a factory, whose props come only from the callee's declaration, which an assertion that replaces the callee's
/// type withholds.
fn differential_declines(case: &Case) -> Option<&'static str> {
    let form = case.id.split_once(" | ").map_or("", |(form, _)| form);
    if differential_subject(form).is_empty() || ARGUMENT_FORMS.contains(&form) {
        return None;
    }
    case.closed.or_else(|| (form == "factory" && !case.keeps).then_some("factory callee withheld"))
}

/// The forms whose wrapped expression is a value handed to a wrapper, not a callee: nothing calls it there, so the
/// closed list says nothing about it, and the value beneath a cast is read as origin/main read it.
const ARGUMENT_FORMS: &[&str] = &["Object.assign argument", "memo argument"];

/// Review round 10: the forms the review named, a generic wrapper given its props in its type arguments through a
/// replacing cast (`(wrap2 as Foo)<E, P>(Base)`) and a cast on the value a wrapper or `Object.assign` receives
/// (`Object.assign(Impl as unknown as Other, { Item })`, `memo(Impl as Wrapped)`). Each row that does not decline
/// extracts exactly what origin/main extracts for the form's plain, uncast call. Origin/main itself read some casts
/// differently from its own plain call: `(wrap2 as typeof wrap2)<E, P>(Base)` as a factory of `wrap2` (`Button:wrap2`,
/// as it did not peel a callee for the wrapper rule), and `Object.assign(Impl as X, { Item })` without `Item` (as it
/// did not peel the first argument when it attached inline members). Those rows are counted, by form.
#[test]
fn the_cast_forms_round_10_names_extract_what_origin_main_extracts_for_the_plain_call() {
    let base = base_detections();
    // Round 11 adds `React.createContext<Props>(Default)`, which is no component through any cast.
    let forms = ["wrapper with two type arguments", "Object.assign argument", "memo argument", "context"];
    let mut failures = Vec::new();
    let mut main_differs_from_its_plain_call = [0; 4];
    for case in differential_cases() {
        let form = case.id.split_once(" | ").map_or("", |(form, _)| form);
        let Some(index) = forms.iter().position(|named| *named == form) else { continue };
        let plain = base.get(format!("{form} | plain").as_str()).copied().unwrap_or("not in the base table");
        let main = base.get(case.id.as_str()).copied().unwrap_or("not in the base table");
        let actual = differential_detections(&case.source);
        if differential_declines(&case).is_some() {
            if actual.split(' ').any(|d| d.starts_with(differential_subject(form))) {
                failures.push(format!("{} declines: main `{main}`, now `{actual}`", case.id));
            }
            continue;
        }
        if actual != plain {
            failures.push(format!("{}: main's plain call `{plain}`, main `{main}`, now `{actual}`", case.id));
        }
        if main != plain {
            main_differs_from_its_plain_call[index] += 1;
        }
    }
    assert!(failures.is_empty(), "{} rows:\n{}", failures.len(), failures.join("\n"));
    assert_eq!(main_differs_from_its_plain_call, [101, 112, 0, 0]);
}

/// Review round 11, finding 2: a namespace read for its name (`React` in `React.createContext`, `Object.assign`,
/// `React.Component`, an imported namespace's member) has no evidence of its own; an unrelated call signature on it
/// does not hide the member, whose own type alone says what calling it gives.
#[test]
fn a_namespace_read_for_its_name_has_no_evidence_of_its_own() {
    let shape = "(React as unknown as { (): void; {m}: typeof React.{m} })";
    let names_of = |source: &str| names(source);
    let context = |receiver: &str| {
        names_of(&format!(
            "declare const Default: Props;\nexport const Context = {receiver}.createContext<Props>(Default);"
        ))
    };
    assert_eq!(context("React"), Vec::<String>::new());
    assert_eq!(context(&shape.replace("{m}", "createContext")), Vec::<String>::new());
    let object_member = "(React as { createContext: (d: unknown) => object })";
    assert_eq!(context(object_member), Vec::<String>::new());
    // The same receiver in front of the other members read by name: each equals its plain form.
    let class = |receiver: &str| {
        tsx(&format!("export class Button extends {receiver}.Component<Props> {{ render() {{ return <div />; }} }}"))
    };
    let plain_class = class("React");
    assert_eq!(plain_class.len(), 1, "{plain_class:#?}");
    assert_eq!(class(&shape.replace("{m}", "Component")), plain_class);
    let assign = |receiver: &str| {
        names_of(&format!(
            "function Impl(props: Props) {{ return <div />; }}\n\
             export const Root = {receiver}.assign(Impl, {{ Item: (props: Props) => <div /> }});"
        ))
    };
    assert_eq!(assign("Object"), ["Item", "Root"]);
    assert_eq!(assign("(Object as unknown as { (): void; assign: typeof Object.assign })"), ["Item", "Root"]);
    let imported =
        |receiver: &str| names_of(&format!("import * as Ui from './ui';\nexport const Button = {receiver}.Button;"));
    assert_eq!(imported("Ui"), ["Button"]);
    assert_eq!(imported("(Ui as unknown as { (): void; Button: unknown })"), ["Button"]);
    let rendered = |receiver: &str| {
        names_of(&format!(
            "function Foo(props: Props, ref: unknown) {{ return <div />; }}\n\
             export const el = {receiver}.createElement(Foo);"
        ))
    };
    assert_eq!(rendered("React"), ["Foo"]);
    assert_eq!(rendered(&shape.replace("{m}", "createElement")), ["Foo"]);
}

/// Review round 11, finding 1: each type declaration the alias walk meets visits only the aliases of its name, so N
/// aliases and N generic functions cost work proportional to N, not N squared.
#[test]
fn the_typeof_alias_walk_grows_with_the_declarations_not_with_the_aliases() {
    let work = |n: usize| {
        let mut source = String::from(HEADER);
        for i in 0..n {
            source.push_str(&format!(
                "declare function make{i}(): unknown;\ntype Factory{i} = typeof make{i};\nfunction g{i}<T>(t: T) {{ return t; }}\n"
            ));
        }
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &source, oxc_span::SourceType::tsx()).parse();
        super::component::steps::take_examined();
        assert_eq!(super::component::typeof_aliases(&parsed.program).len(), n);
        super::component::steps::take_examined()
    };
    // Per alias: its own declaration (a lookup and its slot) and a type parameter (a lookup); the header's five
    // imported names and its interface are a lookup each.
    assert_eq!(work(10), 6 + 3 * 10);
    assert_eq!(work(20) - 6, 2 * (work(10) - 6));
    assert_eq!(work(40) - 6, 2 * (work(20) - 6));
}

/// The forms where origin/main read a cast differently from its own plain call; the differential holds their rows to
/// main's plain call instead (`the_cast_forms_round_10_names_extract_what_origin_main_extracts_for_the_plain_call`).
const MAIN_DIFFERS_FROM_ITS_PLAIN_CALL: &[&str] = &["wrapper with two type arguments", "Object.assign argument"];

/// What origin/main (f4a88f93, oxc 0.158) extracts from each of `differential_cases()`, one `id<TAB>detections` line
/// each, as `differential_detections` prints it. Produced by copying the block between `BEGIN DIFFERENTIAL CASES` and
/// `END DIFFERENTIAL CASES` into a scratch test module on origin/main that prints each line; regenerate it the same way
/// when a case changes, never by hand.
fn base_detections() -> FxHashMap<&'static str, &'static str> {
    include_str!("wrapper_matrix_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect()
}

/// No recall regression: every row the base (origin/main, oxc 0.158) detected is detected the same, unless its call's
/// whole result is on the closed list or a factory's callee lost the declaration its props come from
/// (`differential_declines`); and every row that declines finds no candidate.
#[test]
fn every_form_the_base_detected_is_still_detected_unless_its_result_is_on_the_closed_list() {
    let base = base_detections();
    let cases = differential_cases();
    assert_eq!(cases.len(), base.len(), "the base table and the cases are out of step");
    let mut failures = Vec::new();
    for case in &cases {
        let Some(&row) = base.get(case.id.as_str()) else { panic!("{} is not in the base table", case.id) };
        let form = case.id.split_once(" | ").map_or("", |(form, _)| form);
        let expected = match base.get(format!("{form} | plain").as_str()) {
            Some(plain) if MAIN_DIFFERS_FROM_ITS_PLAIN_CALL.contains(&form) => plain,
            Some(_) | None => row,
        };
        let subject = differential_subject(form);
        let actual = differential_detections(&case.source);
        let found: Vec<&str> = actual.split(' ').collect();
        let declined = differential_declines(case);
        // Through a cast the base did not read `Object.assign` at all and published `Impl` under its own name; read
        // now, `Root` aliases it.
        let kept = |detection: &str| {
            found.contains(&detection)
                || (detection == "Impl:Props" && declined.is_none() && found.contains(&"Root:Props"))
        };
        let holds = actual != "parse error"
            && expected.split(' ').filter(|d| *d != "-" && !(declined.is_some() && d.starts_with(subject))).all(kept)
            && !(declined.is_some() && found.iter().any(|d| d.starts_with(subject)));
        if !holds {
            failures.push(format!(
                "{} (declined: {declined:?}): base `{expected}`, now `{actual}`\n{}",
                case.id, case.source
            ));
        }
    }
    assert!(failures.is_empty(), "{} rows:\n\n{}", failures.len(), failures.join("\n\n"));
}

/// The reverse: each closed-list entry, as every form's whole call result in every shape, declines the form's
/// candidate, and nothing else declines a form but a factory's withheld callee.
#[test]
fn each_closed_list_entry_declines_every_form() {
    let cases = differential_cases();
    for entry in CLOSED_LIST {
        let rows: Vec<&Case> = cases.iter().filter(|case| case.closed == Some(*entry)).collect();
        // A function and a callable literal for every form, and a namespace method for every member callee.
        assert!(rows.len() > DIFFERENTIAL_FORMS.len() * 2, "{entry}: {} rows", rows.len());
        for case in rows {
            let form = case.id.split_once(" | ").map_or("", |(form, _)| form);
            let subject = differential_subject(form);
            // A form with no candidate of its own, or whose cast value is handed on rather than called.
            if differential_declines(case).is_none() {
                continue;
            }
            let actual = differential_detections(&case.source);
            assert!(!actual.split(' ').any(|d| d.starts_with(subject)), "{}: `{actual}`\n{}", case.id, case.source);
        }
    }
    let base = base_detections();
    let declined: Vec<String> = cases
        .iter()
        .filter_map(|case| {
            let subject = differential_subject(case.id.split_once(" | ").map_or("", |(form, _)| form));
            let detected = base.get(case.id.as_str()).is_some_and(|d| d.split(' ').any(|d| d.starts_with(subject)));
            differential_declines(case).filter(|_| detected).map(|why| format!("{}: {why}", case.id))
        })
        .collect();
    // Every base detection given up, by reason: pinned so a new decline is a reviewed change.
    let count = |why: &str| declined.iter().filter(|row| row.ends_with(&format!(": {why}"))).count();
    let by_reason = ["object", "symbol", "void", "factory callee withheld"].map(|why| (why, count(why)));
    assert_eq!(by_reason.iter().map(|(_, n)| n).sum::<usize>(), declined.len(), "{}", declined.join("\n"));
    assert_eq!(
        by_reason,
        [("object", 52), ("symbol", 20), ("void", 40), ("factory callee withheld", 93)],
        "{}",
        declined.join("\n")
    );
}

/// The closed list is one exhaustive match over the type node enum: no catch-all arm, so a new OXC type node kind does
/// not compile until it is placed, and exactly three entries decline.
#[test]
fn the_closed_list_is_one_match_without_a_catch_all_arm() {
    let source = include_str!("component.rs");
    let start = source.find("fn never_renders(").expect("never_renders is defined in component.rs");
    let body = &source[start..start + source[start..].find("\n}\n").expect("never_renders ends")];
    for line in body.lines().filter(|line| line.contains("=>")) {
        let arm = line.trim_start().trim_start_matches("| ");
        assert!(arm.starts_with("TSType::"), "an arm that is not a TSType variant: `{line}`");
    }
    assert_eq!(body.matches(") => true").count(), 3, "the closed list is object, symbol and void:\n{body}");
}

// Review round 9: an instantiation at an initializer or an argument (`memo(List<Row>)`, `const RowList = List<Row>`)
// is read as origin/main reads it. Binding its type arguments is a follow-up with its own model.

// BEGIN PARITY CASES. Copied verbatim into a scratch test on origin/main (oxc 0.158) to print
// `instantiation_parity_base.tsv`; keep the two in step.

const PARITY_PRELUDE: &str = "import React, { memo } from 'react';\n\
                              interface ListProps<T> { items?: T[] }\n\
                              interface ItemProps { size?: string }\n\
                              interface CardProps { title?: string }\n\
                              declare function withTheme<C>(c: C): C;\n\
                              function List<T>(props: ListProps<T>) { return <div />; }\n\
                              function View<P>(props: P) { return <div />; }\n";

/// An initializer or argument site with an instantiation, and a control without one.
const PARITY_CASES: &[(&str, &str)] = &[
    ("alias", "export const RowList = List<string>;"),
    ("alias, no instantiation", "export const RowList = List;"),
    ("alias chain", "export const RowList = List<string>;\nexport const PublicList = RowList;"),
    ("alias chain, no instantiation", "export const RowList = List;\nexport const PublicList = RowList;"),
    ("default export", "export default List<string>;"),
    ("default export, no instantiation", "export default List;"),
    ("props type is the type parameter", "export const CardView = View<CardProps>;"),
    ("memo", "export const RowList = memo(List<string>);"),
    ("memo, no instantiation", "export const RowList = memo(List);"),
    ("wrapper", "export const RowList = withTheme(List<string>);"),
    ("Object.assign", "export const RowList = Object.assign(List<string>, { Item: (props: ItemProps) => <div /> });"),
    ("parenthesized", "export const RowList = (List<string>);"),
    ("non-null", "export const RowList = (List<string>)!;"),
    ("as any", "export const RowList = (List<string>) as any;"),
    ("satisfies", "export const RowList = (List<string>) satisfies unknown;"),
    ("asserted beneath", "export const RowList = (List as any)<string>;"),
    ("imported", "import { Grid } from './grid';\nexport const RowGrid = Grid<string>;"),
    ("FC annotation", "export const Button: React.FC<CardProps> = ((<T,>({ title = 'x' }) => <div />)<string>)!;"),
];

/// Every mapping of a row (name, props type and arguments, type parameters, rule) and every diagnostic, sorted.
fn parity_output(source: &str) -> String {
    let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), &format!("{PARITY_PRELUDE}{source}"));
    let mut lines: Vec<String> = data
        .component_mappings
        .iter()
        .map(|m| {
            format!(
                "{}:{}{}{}{}",
                m.component_name,
                m.props_type_name,
                serde_json::to_string(&m.props_type_args).unwrap_or_default(),
                serde_json::to_string(&m.type_params).unwrap_or_default(),
                serde_json::to_string(&m.rule).unwrap_or_default()
            )
        })
        .chain(data.diagnostics.iter().map(|d| format!("{:?} {:?}: {}", d.severity, d.code, d.message)))
        .collect();
    lines.sort();
    lines.join(" | ")
}

// END PARITY CASES.

/// Parity with origin/main (f4a88f93, oxc 0.158) at the sites where OXC 0.159 changed nothing: what each row extracts
/// and reports, exactly. Regenerate `instantiation_parity_base.tsv` from a scratch test on origin/main that prints
/// `id<TAB>parity_output(source)` for each case, never by hand.
#[test]
fn an_instantiated_value_is_read_as_origin_main_reads_it() {
    let base: FxHashMap<&str, &str> =
        include_str!("instantiation_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    assert_eq!(base.len(), PARITY_CASES.len(), "the base table and the cases are out of step");
    let failures: Vec<String> = PARITY_CASES
        .iter()
        .filter_map(|(id, source)| {
            let now = parity_output(source);
            let main = base.get(id).copied().unwrap_or("not in the base table");
            (main != now).then(|| format!("{id}:\n  main {main}\n  now  {now}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Finding 1 of round 9, and round 10's one walker: every `typeof` alias's declarations are counted and its shadowing
/// scopes found in one walk of the module, however many aliases there are; a module with none is not walked.
#[test]
fn every_typeof_alias_is_counted_in_one_walk_of_the_module() {
    let cost = |aliases: usize| {
        let mut source = String::from(HEADER);
        for i in 0..aliases {
            source.push_str(&format!("declare function make{i}(): unknown;\ntype Factory{i} = typeof make{i};\n"));
        }
        source.push_str("export const Button = memo((props: Props) => <div />);\n");
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &source, oxc_span::SourceType::tsx()).parse();
        super::component::steps::take_walks();
        let kept = super::component::typeof_aliases(&parsed.program).len();
        (kept, super::component::steps::take_walks())
    };
    assert_eq!(cost(0), (0, 0));
    assert_eq!(cost(1), (1, 1));
    assert_eq!(cost(10), (10, 1));
    assert_eq!(cost(40), (40, 1));
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
        "export as namespace Factory;",
        "try {} catch (e) { type Factory = () => string; }",
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
        "export declare type Factory = () => string;",
        "import Factory = require('./factory');",
        "import type Factory = require('./factory');",
        "export default interface Factory { (): string }",
        "export default class Factory {}",
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
            ["'Button' is built by a call to 'createButton' through a type assertion that replaces its type, so the \
                 declaration that would give its props is not read and it is not listed"],
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
        assert_eq!(tsx(&source), plain, "{callee}");
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

/// Finding 6, as the output shows it: an element factory asserted to give a result on the closed list renders nothing,
/// so a ref-taking function passed only to it is a render callback, as when nothing renders it. A string is a node, off
/// the list, and no evidence.
#[test]
fn an_element_factory_asserted_to_a_closed_list_result_renders_nothing() {
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
    assert_eq!(names("(createElement as (t: any) => string)"), ["Foo"]);
    assert_eq!(names("(createElement as (t: any) => void)"), Vec::<String>::new());
    assert_eq!(names("(createElement as (t: any) => object)"), Vec::<String>::new());
    assert_eq!(names("String"), Vec::<String>::new());
}

/// Finding 6's sibling: a function returning an element factory's result asserted to a closed-list type returns no
/// element.
#[test]
fn a_function_returning_a_factory_result_asserted_to_a_closed_list_type_returns_no_element() {
    let names = |call: &str| {
        tsx(&format!("import {{ createElement }} from 'react';\nexport const Button = (props: Props) => {call};"))
            .iter()
            .map(|m| m["component_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(names("createElement('div')"), ["Button"]);
    assert_eq!(names("(createElement as (t: any) => React.ReactElement)('div')"), ["Button"]);
    assert_eq!(names("(createElement as (t: any) => string)('div')"), ["Button"]);
    assert_eq!(names("(createElement as (t: any) => symbol)('div')"), Vec::<String>::new());
    assert_eq!(names("([1].map as (f: unknown) => string)(() => <div />)"), ["Button"]);
    assert_eq!(names("([1].map as (f: unknown) => void)(() => <div />)"), Vec::<String>::new());
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
        // Round 10: one walker for every kind.
        ("enum E { A = (make as Factory)() }", Some(true)),
        ("function g() { type Factory = string; enum E { A = (make as Factory)() } }", Some(false)),
        ("enum E { A = (<Factory,>() => (make as Factory)())() }", Some(false)),
        ("try {} catch (e) { type Factory = string; const v = (make as Factory)(); }", Some(false)),
        ("try {} catch (e) { type Factory = string; }\nconst v = (make as Factory)();", Some(true)),
        ("function g(v = (make as Factory)()) { type Factory = string; }", Some(true)),
        ("const g = (v = (make as Factory)()) => { type Factory = string; };", Some(true)),
        ("function g<Factory>(v = (make as Factory)()) {}", Some(false)),
        ("class K { static { type Factory = string; (make as Factory)(); } }", Some(false)),
        ("class K { static { type Factory = string; } m() { return (make as Factory)(); } }", Some(true)),
        (
            "declare module 'm' { import Factory from 'f'; export function h(): typeof Factory; }\n(make as Factory)();",
            Some(true),
        ),
        ("export = make;\nconst v = (make as Factory)();", Some(true)),
        ("export declare type Factory = string;\nconst v = (make as Factory)();", None),
        ("import Factory = require('f');\nconst v = (make as Factory)();", None),
        ("import type Factory = require('f');\nconst v = (make as Factory)();", None),
        ("export default interface Factory {}\nconst v = (make as Factory)();", None),
        ("export default class Factory {}\nconst v = (make as Factory)();", None),
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
    assert_eq!(rendered("(createElement as (t: any) => string)"), 1);
    assert_eq!(rendered("(createElement as (t: any) => number | boolean)"), 1);
    assert_eq!(rendered("(React as { createElement: (t: any) => string }).createElement"), 1);
    assert_eq!(rendered("(createElement as (t: any) => object)"), 0);
    assert_eq!(rendered("(createElement as (t: any) => void)"), 0);
    assert_eq!(rendered("(React as { createElement: (t: any) => symbol }).createElement"), 0);
    // A component wrapper renders its argument by its name, whatever it returns.
    assert_eq!(rendered("memo"), 1);
}

/// Round 8: a rendering factory called through an assertion that replaces its type is not listed, as its props come
/// only from the declaration the assertion withholds; the skip says so, once. A result on the closed list says the
/// call gives no component and keeps the generic skip; the plain call is listed with no skip.
#[test]
fn a_factory_called_through_an_assertion_says_why_it_is_not_listed() {
    let outcome = |callee: &str| {
        let source = format!(
            "{HEADER}declare const other: unknown;\n\
             function createButton() {{ return (props: Props) => <div />; }}\n\
             export const Button = {callee}();"
        );
        let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), &source);
        let names: Vec<String> = data.component_mappings.iter().map(|m| m.component_name.clone()).collect();
        let skips: Vec<String> = data
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::SkippedCandidate)
            .map(|d| d.message.clone())
            .collect();
        (names, skips)
    };
    assert_eq!(outcome("createButton"), (vec!["Button".to_owned()], Vec::new()));
    assert_eq!(outcome("(createButton as typeof createButton)"), (vec!["Button".to_owned()], Vec::new()));
    let withheld = "'Button' is built by a call to 'createButton' through a type assertion that replaces its type, \
                    so the declaration that would give its props is not read and it is not listed";
    for callee in [
        "(createButton as F)",
        "(createButton as typeof other)",
        "(createButton as object)",
        "(createButton as () => React.FC<Props>)",
        "(createButton as () => React.ComponentType<Props>)",
        "(createButton as () => React.FC<Props> | null)",
        "(createButton as { (): React.FC<Props> })",
        "(createButton as () => string)",
        "(createButton as () => React.ReactNode)",
        "(createButton as () => unknown)",
    ] {
        assert_eq!(outcome(callee), (Vec::new(), vec![withheld.to_owned()]), "{callee}");
    }
    let generic = "'Button' is a PascalCase binding but matched no known component pattern (FC annotation, forwardRef, \
                   HOC wrapper, or identifier alias)";
    for callee in ["(createButton as () => object)", "(createButton as () => void)"] {
        assert_eq!(outcome(callee), (Vec::new(), vec![generic.to_owned()]), "{callee}");
    }
}

/// The components a `.tsx` source after `HEADER` publishes, by name, in order.
fn names(source: &str) -> Vec<String> {
    tsx(source).iter().map(|m| m["component_name"].as_str().unwrap().to_owned()).collect()
}

/// Round 7, finding A: `any[]` is assignable to `ReactNode` (an iterable of `any`), so a namespace method asserted to
/// return it is no evidence against `map`, and the callback's JSX is the function's result.
#[test]
fn a_map_asserted_to_return_any_array_still_renders_its_callback() {
    let list = |map: &str| {
        names(&format!(
            "export const List = ({{ rows }}: {{ rows: string[] }}) => (rows as {{ map(render: (row: string) => \
             React.ReactNode): {map} }}).map(row => <span>{{row}}</span>);"
        ))
    };
    assert_eq!(list("any[]"), ["List"]);
    assert_eq!(list("unknown[]"), ["List"]);
    assert_eq!(list("object"), Vec::<String>::new());
}

/// Round 7, finding B: a wrapper whose callee is asserted to another signature has no declaration to read for the
/// props its result adds, but the props type it is called with still names the component's props.
#[test]
fn an_asserted_wrapper_keeps_the_props_its_type_arguments_name() {
    let mapping = |callee: &str| {
        let source = format!(
            "{HEADER}interface PublicProps {{ label?: string }}\n\
             declare const ui: {{ withContext<P>(View: React.ComponentType<P>): React.FC<P> }};\n\
             function Base(props: {{ inner?: string }}) {{ return <div />; }}\n\
             export const Public = {callee}<PublicProps>(Base);"
        );
        let data = parse_file(Utf8Path::new("/fixtures/Button.tsx"), &source);
        data.component_mappings
            .iter()
            .find(|m| m.component_name == "Public")
            .map(|m| (m.props_type_name.to_string(), m.wrapper_callee.as_ref().map(|c| c.name.to_string())))
    };
    assert_eq!(mapping("ui.withContext"), Some(("PublicProps".to_owned(), Some("ui.withContext".to_owned()))));
    assert_eq!(
        mapping("(ui as { withContext<P>(View: React.ComponentType<P>): React.FC<P> }).withContext"),
        Some(("PublicProps".to_owned(), None))
    );
    assert_eq!(mapping("(ui as { withContext(View: unknown): object }).withContext"), None);
}

/// Round 7, finding C: an object type is no evidence that a call gives no element, as React's own element is one
/// (`{ type; props; key }`): only the closed list is.
#[test]
fn a_factory_asserted_to_an_element_shaped_object_still_renders() {
    let div = |result: &str| {
        names(&format!(
            "export const Div = (props: Props) => (React as {{ createElement(tag: 'div'): {result} }}).createElement('div');"
        ))
    };
    assert_eq!(div("{ type: string; props: {}; key: string | null }"), ["Div"]);
    assert_eq!(div("{}"), ["Div"]);
    assert_eq!(div("object"), Vec::<String>::new());
}

/// Review round 12: an `extends` target is constructed, so its construct signatures say what it gives; a callee is
/// called, so its call signatures do; a namespace member is read with the kind its site uses. A signature of the
/// other kind is no evidence either way.
#[test]
fn a_heritage_reads_construct_signatures_and_a_call_reads_call_signatures() {
    let class = |target: &str| {
        tsx(&format!("export class Button extends {target}<Props> {{ render() {{ return <div />; }} }}"))
    };
    let plain_class = class("React.Component");
    assert_eq!(plain_class.len(), 1, "{plain_class:#?}");
    for target in [
        "(React.Component as unknown as { new <P>(props: P): React.Component<P>; (): void })",
        "(React.Component as unknown as new <P>(props: P) => React.Component<P>)",
        "(React.Component as unknown as () => object)",
        "(React as unknown as { Component: { new <P>(props: P): React.Component<P>; (): void } }).Component",
        "(React as unknown as { Component(): void }).Component",
    ] {
        assert_eq!(class(target), plain_class, "{target}");
    }
    for target in [
        "(React.Component as unknown as { new (props: unknown): object; (): React.FC<Props> })",
        "(React.Component as unknown as new (props: unknown) => void)",
        "(React as unknown as { Component: new (props: unknown) => symbol }).Component",
    ] {
        assert_eq!(class(target), Vec::<serde_json::Value>::new(), "{target}");
    }
    let call = |callee: &str| tsx(&format!("export const Button = {callee}((props: Props) => <div />);"));
    let plain_call = call("memo");
    assert_eq!(plain_call.len(), 1, "{plain_call:#?}");
    for callee in [
        "(memo as unknown as { new (c: unknown): object; (c: unknown): React.FC<Props> })",
        "(memo as unknown as new (c: unknown) => object)",
    ] {
        assert_eq!(call(callee), plain_call, "{callee}");
    }
    assert_eq!(
        call("(memo as unknown as { new (c: unknown): React.FC<Props>; (c: unknown): object })"),
        Vec::<serde_json::Value>::new()
    );
}
