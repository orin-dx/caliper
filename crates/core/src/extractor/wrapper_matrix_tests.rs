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
        ("(Object as (value: object) => { id: string })", false),
        ("(Object as (value: object) => unknown[])", false),
        ("(Object as (value: object) => object | null)", false),
        ("(Object as (value: object) => object & React.ReactElement)", false),
        ("(Object as unknown as (value: object) => string)", false),
        ("(Object as (value: object) => number)", false),
        ("(Object as (value: object) => boolean)", false),
        ("(Object as (value: object) => string | null)", false),
        ("(Object as (value: object) => null)", false),
        ("(Object as (value: object) => string | object)", false),
        ("(Object as (value: object) => string[])", false),
        ("(Object as (value: object) => React.ReactElement)", false),
        ("(Object as (value: object) => React.ReactElement | null)", false),
        ("(Object as F)", false),
        ("(Object as typeof Object)", false),
        ("(Object as any)", false),
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

/// Review round 4, finding 1: a local alias of `typeof` the callee is the callee's own type, so a generic wrapper called
/// through it keeps the callee whose result adds props (`wrapper_callee`); any other alias is a cast that replaces it.
#[test]
fn a_factory_asserted_to_an_alias_of_its_own_type_is_read_through_its_declaration() {
    let factory = "function createButton<T>(render: React.FC<T>): React.FC<T> { return render; }\n";
    let plain = tsx(&format!("{factory}export const Button = createButton<Props>((props: Props) => <div />);"));
    assert_eq!(plain.len(), 1, "{plain:#?}");
    assert_eq!(plain[0]["component_name"], "Button");
    // A cast that replaces the callee's type keeps the component without the callee whose result adds props.
    let replaced = tsx(&format!(
        "{factory}export const Button = (createButton as <T>(render: React.FC<T>) => React.FC<T>)<Props>((props: Props) => <div />);"
    ));
    assert_ne!(replaced, plain);
    for alias in ["type Factory = typeof createButton;", "export type Factory = (typeof createButton);"] {
        let source = format!(
            "{factory}{alias}\nexport const Button = (createButton as Factory)<Props>((props: Props) => <div />);"
        );
        assert_eq!(tsx(&source), plain, "{source}");
    }
    // An alias of another value's type, a generic alias and one declared twice are not the identity.
    for alias in [
        "declare const other: unknown; type Factory = typeof other;",
        "type Factory<T = unknown> = typeof createButton;",
        "type Factory = typeof createButton; namespace N {} type Factory = string;",
    ] {
        let source = format!(
            "{factory}{alias}\nexport const Button = (createButton as Factory)<Props>((props: Props) => <div />);"
        );
        assert_eq!(tsx(&source), replaced, "{source}");
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
    // Round 13: React's factories that build no component, given a component and the props type as a wrapper is.
    (
        "createElement call",
        "React.createElement",
        "function Foo(props: Props) { return <div />; }\ndeclare const props: Props;\n\
         export const Button = {C}<Props>(Foo, props);",
    ),
    (
        "cloneElement call",
        "React.cloneElement",
        "function Foo(props: Props) { return <div />; }\nexport const Button = {C}<Props>(Foo);",
    ),
    (
        "createRef call",
        "React.createRef",
        "function Foo(props: Props) { return <div />; }\nexport const Button = {C}<Props>(Foo);",
    ),
    (
        "lazy call",
        "React.lazy",
        "function Foo(props: Props) { return <div />; }\nexport const Button = {C}<Props>(Foo);",
    ),
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
    case.closed
}

/// The forms calling React's `createElement`, `cloneElement` or `createRef` as a generic wrapper is called. Parity
/// (round 17): each row extracts exactly what origin/main extracts for that row. Main documented the plain call and
/// every cast it read through as a component (a known wrong answer, left for the follow-up); a cast main read no callee
/// through (an assertion, `satisfies`, `!`) stays declined.
const NO_COMPONENT_FACTORY_FORMS: &[&str] = &["createElement call", "cloneElement call", "createRef call"];

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
    let forms = [
        "wrapper with two type arguments",
        "Object.assign argument",
        "memo argument",
        "context",
        "createElement call",
        "cloneElement call",
        "createRef call",
        "lazy call",
    ];
    let mut failures = Vec::new();
    let mut main_differs_from_its_plain_call = [0; 8];
    let mut main_documented_no_component = [0; 8];
    for case in differential_cases() {
        let form = case.id.split_once(" | ").map_or("", |(form, _)| form);
        let Some(index) = forms.iter().position(|named| *named == form) else { continue };
        let plain = base.get(format!("{form} | plain").as_str()).copied().unwrap_or("not in the base table");
        let main = base.get(case.id.as_str()).copied().unwrap_or("not in the base table");
        let actual = differential_detections(&case.source);
        let subject = differential_subject(form);
        if NO_COMPONENT_FACTORY_FORMS.contains(&form) && differential_declines(&case).is_none() {
            if actual != main {
                failures.push(format!("{}: main `{main}`, now `{actual}`", case.id));
            }
            if main.split(' ').any(|d| d.starts_with(subject)) {
                main_documented_no_component[index] += 1;
            }
            continue;
        }
        if differential_declines(&case).is_some() {
            if actual.split(' ').any(|d| d.starts_with(subject)) {
                failures.push(format!("{} declines: main `{main}`, now `{actual}`", case.id));
            }
            if main.split(' ').any(|d| d.starts_with(subject)) {
                main_documented_no_component[index] += 1;
            }
            continue;
        }
        // Main's plain call, or main's own row where a cast is left to the rule main ran (round 19).
        if actual != plain && actual != main {
            failures.push(format!("{}: main's plain call `{plain}`, main `{main}`, now `{actual}`", case.id));
        }
        if main != plain {
            main_differs_from_its_plain_call[index] += 1;
        }
    }
    assert!(failures.is_empty(), "{} rows:\n{}", failures.len(), failures.join("\n"));
    // `React.lazy<Props>(Foo)`: main read the plain call as a wrapper and missed it through every cast.
    assert_eq!(main_differs_from_its_plain_call, [101, 112, 0, 0, 0, 0, 0, 101]);
    // Rows where main published `Button`, by form: the closed-list rows it read through, and every row it read as the
    // plain call to a factory that builds no component.
    assert_eq!(main_documented_no_component, [11, 0, 0, 0, 30, 30, 30, 6]);
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
        assert_eq!(
            super::component::typeof_aliases(&parsed.program, &super::component::file_names(&parsed.program)).len(),
            n
        );
        super::component::steps::take_examined()
    };
    // Per alias, the five names its lines declare (`make`, `Factory`, `g`, `T`, `t`); the header's five imported names
    // and its interface.
    assert_eq!(work(10), 6 + 5 * 10);
    assert_eq!(work(20) - 6, 2 * (work(10) - 6));
    assert_eq!(work(40) - 6, 2 * (work(20) - 6));
}

/// Review round 25, finding 1: N blocks of one namespace cost one lookup each, not N spans per alias.
#[test]
fn many_blocks_of_one_namespace_cost_work_proportional_to_them() {
    let work = |n: usize| {
        let mut source = String::from("declare function wrap(): unknown;\ntype Wrap = typeof wrap;\n");
        for _ in 0..n {
            source.push_str("namespace N { export type Wrap = () => void; }\n");
        }
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &source, oxc_span::SourceType::tsx()).parse();
        super::component::steps::take_examined();
        assert_eq!(
            super::component::typeof_aliases(&parsed.program, &super::component::file_names(&parsed.program)).len(),
            usize::from(n == 0),
            "{n}"
        );
        super::component::steps::take_examined()
    };
    assert_eq!(work(1000), 2 * work(500) - work(0));
    assert_eq!(work(500) - work(0), 500 * (work(1) - work(0)));
}

/// Review round 23: a name the module gives N `typeof` aliases is ambiguous and dropped before the walk: it costs one
/// lookup per declaration, not N per declaration, and no queried name is built for it.
#[test]
fn a_name_given_many_typeof_aliases_costs_work_proportional_to_them() {
    let work = |n: usize| {
        let mut source = String::from(HEADER);
        source.push_str("declare const ns: { make(): unknown };\ntype Other = typeof ns.make;\n");
        for _ in 0..n {
            source.push_str("type Factory = typeof ns.make;\n");
        }
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &source, oxc_span::SourceType::tsx()).parse();
        super::component::steps::take_examined();
        super::component::steps::take_written();
        let aliases = super::component::typeof_aliases(&parsed.program, &super::component::file_names(&parsed.program));
        let mut names: Vec<&str> = aliases.keys().map(|name| name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["Other"], "{n}");
        (super::component::steps::take_examined(), super::component::steps::take_written())
    };
    // The header's six type-space names are a lookup each, `Other` a lookup and its slot, each `Factory` a lookup
    // (`ns` is a value). Only `Other`'s queried name, `ns.make`, is written.
    assert_eq!(work(2), (6 + 2 + 2, 8));
    assert_eq!(work(500), (6 + 2 + 500, 8));
    assert_eq!(work(1000).0 - 8, 2 * (work(500).0 - 8));
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
        // A form where main read a cast differently from its plain call holds to either: main's row (the cast left to
        // the rule main ran) or main's plain call (the cast read as the plain call).
        let plain = match base.get(format!("{form} | plain").as_str()) {
            Some(plain) if MAIN_DIFFERS_FROM_ITS_PLAIN_CALL.contains(&form) => Some(*plain),
            Some(_) | None => None,
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
        let holds_for = |expected: &str| {
            actual != "parse error"
                && expected
                    .split(' ')
                    .filter(|d| *d != "-" && !(declined.is_some() && d.starts_with(subject)))
                    .all(kept)
                && !(declined.is_some() && found.iter().any(|d| d.starts_with(subject)))
        };
        let expected = plain.unwrap_or(row);
        let holds = holds_for(row) || plain.is_some_and(holds_for);
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
    let by_reason = ["object", "symbol", "void"].map(|why| (why, count(why)));
    assert_eq!(by_reason.iter().map(|(_, n)| n).sum::<usize>(), declined.len(), "{}", declined.join("\n"));
    assert_eq!(by_reason, [("object", 64), ("symbol", 24), ("void", 48)], "{}", declined.join("\n"));
}

/// Round 22, the other direction: every detection this head makes that origin/main did not is a cast that keeps the
/// callee's type or a known React wrapper read through a cast, counted by reason and pinned. A cast that replaces a
/// custom wrapper's or factory's type is read as main read it, so it adds nothing.
#[test]
fn every_detection_main_did_not_make_is_an_identity_cast_or_a_known_react_wrapper() {
    let base = base_detections();
    let mut by_reason: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut failures = Vec::new();
    for case in differential_cases() {
        let row = base.get(case.id.as_str()).copied().unwrap_or("-");
        let actual = differential_detections(&case.source);
        let main: Vec<&str> = row.split(' ').collect();
        let added: Vec<&str> = actual.split(' ').filter(|d| *d != "-" && !main.contains(d)).collect();
        if added.is_empty() {
            continue;
        }
        let (form, _) = case.id.split_once(" | ").unwrap_or((case.id.as_str(), ""));
        let plain = base.get(format!("{form} | plain").as_str()).copied().unwrap_or("-");
        let reason = if case.keeps {
            "identity cast"
        } else if KNOWN_REACT_WRAPPER_FORMS.contains(&form) {
            "known React wrapper or API"
        } else if case.closed.is_some() {
            "closed list declines the wrapper, so the component it wraps is listed"
        } else if added.iter().all(|d| plain.split(' ').any(|p| p == *d)) {
            "read as main's own plain call"
        } else {
            failures.push(format!("{}: main `{row}`, now `{actual}`", case.id));
            continue;
        };
        *by_reason.entry(format!("{reason}: {form}")).or_default() += 1;
    }
    assert!(failures.is_empty(), "{} rows:\n{}", failures.len(), failures.join("\n"));
    let summary: Vec<String> = by_reason.iter().map(|(reason, n)| format!("{reason} = {n}")).collect();
    // Round 40 drops the PascalCase call's 8 identity casts and 93 casts read as main's plain call: the callee is the
    // global `Object`, which has no declaration that renders, so through a cast it is no element evidence and every cast
    // row reads as main read it (`a_peeled_part_relies_on_the_built_in_and_on_render_evidence`).
    assert_eq!(
        summary,
        [
            "closed list declines the wrapper, so the component it wraps is listed: Object.assign argument = 11",
            "closed list declines the wrapper, so the component it wraps is listed: wrapper call = 11",
            "identity cast: Object.assign = 12",
            "identity cast: Object.assign argument = 8",
            "identity cast: class = 12",
            "identity cast: element factory = 8",
            "known React wrapper or API: class = 126",
            "known React wrapper or API: element factory = 93",
            "read as main's own plain call: Object.assign = 111",
            "read as main's own plain call: Object.assign argument = 93",
        ]
    );
}

/// The forms built on React's own API, a class's `React.Component` base or `React.createElement` in a component's
/// body, whose component is found through any cast but the closed list.
const KNOWN_REACT_WRAPPER_FORMS: &[&str] = &["class", "element factory"];

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

// BEGIN FACTORY PARITY CASES. Copied verbatim into a scratch test on origin/main (oxc 0.158) to print
// `factory_parity_base.tsv`; keep the two in step.

/// A known factory's provenance: its name, the declaration that binds it, the callee as called (a member callee
/// includes its namespace).
const FACTORY_PROVENANCE: &[(&str, &str, &str)] = &[
    ("named import", "import { createElement } from 'react';", "createElement"),
    ("namespace import", "import * as R from 'react';", "R.createElement"),
    ("default import", "import React from 'react';", "React.createElement"),
    (
        "destructured from the namespace",
        "import React from 'react';\nconst { createElement } = React;",
        "createElement",
    ),
    ("renamed from the namespace", "import React from 'react';\nconst { createElement: h } = React;", "h"),
    ("aliased member", "import React from 'react';\nconst ce = React.createElement;", "ce"),
    ("aliased import", "import { createElement } from 'react';\nconst ce = createElement;", "ce"),
    ("destructured require", "const { createElement } = require('react');", "createElement"),
    ("required namespace", "const R = require('react');", "R.createElement"),
    ("local function", "function createElement(tag: string, ...rest: unknown[]) { return tag; }", "createElement"),
    ("relative module", "import { createElement } from './widgets';", "createElement"),
    ("Preact's h", "import { h } from 'preact';", "h"),
    ("emotion's jsx", "import { jsx } from '@emotion/react';", "jsx"),
    ("relative jsx", "import { jsx } from './widgets';", "jsx"),
];

/// The call written through each cast kind, with `{f}` for the callee.
const FACTORY_CASTS: &[(&str, &str)] = &[
    ("plain", "{f}"),
    ("typeof itself", "({f} as typeof {f})"),
    ("alias of typeof itself", "({f} as Alias)"),
    ("any", "({f} as any)"),
    ("unknown", "({f} as unknown)"),
];

/// Every (provenance, use, cast) row: a function rendering the factory's result, and a binding of a generic call to
/// it with a component and the props type, as a wrapper is called. Then the parameter-default row of finding 3.
fn factory_parity_cases() -> Vec<(String, String)> {
    let mut cases = Vec::new();
    let uses = [
        ("rendered", "export const Button = (props: Props) => {C}('button', null, props.label);"),
        ("wrapper", "function Base(props: Props) { return <div />; }\nexport const Button = {C}<Props>(Base);"),
    ];
    for (provenance, binding, callee) in FACTORY_PROVENANCE {
        for (usage, template) in uses {
            for (cast, wrapped) in FACTORY_CASTS {
                let source = format!(
                    "{binding}\ninterface Props {{ label?: string }}\ntype Alias = typeof {callee};\n{}",
                    template.replace("{C}", &wrapped.replace("{f}", callee))
                );
                cases.push((format!("{provenance} | {usage} | {cast}"), source));
            }
        }
    }
    cases.push((
        "a body declaration does not reach a parameter default".to_owned(),
        "import React from 'react';\ninterface Props { label?: string }\n\
         function Foo(props: Props, ref: unknown) { return <div />; }\n\
         export function build(el = React.createElement(Foo)) { const React = {}; return el; }"
            .to_owned(),
    ));
    // Round 16: the nearest declaration of a name wins whatever its kind, and an alias's origin is read where the
    // alias is declared. `Foo` takes a ref, so it is a component only when something renders it.
    let foo = "import React from 'react';\ninterface Props { label?: string }\n\
               function Foo(props: Props, ref: unknown) { return <div />; }\n";
    for (id, body) in [
        (
            "nearest: an inner factory binding inside a parameter",
            "export function run(createElement: unknown) { { const { createElement } = React; return createElement(Foo); } }",
        ),
        (
            "nearest: an inner parameter inside a factory binding",
            "export function run() { const { createElement } = React; return ((createElement: (c: unknown) => unknown) => createElement(Foo))(String); }",
        ),
        (
            "nearest: an inner factory binding inside a module function of the name",
            "function createElement(tag: unknown) { return tag; }\nexport function run() { const { createElement } = React; return createElement(Foo); }",
        ),
        (
            "nearest: an inner function of the name inside a factory binding",
            "const { createElement } = React;\nexport function run() { function createElement(tag: unknown) { return tag; } return createElement(Foo); }",
        ),
        (
            "nearest: a parameter, after an inner factory binding",
            "export function run(createElement: (c: unknown) => unknown) { { const { createElement } = React; } return createElement(Foo); }",
        ),
        (
            "nearest: an inner factory binding inside a module factory binding",
            "const ce = React.createElement;\nexport function run() { const { createElement: ce } = React; return ce(Foo); }",
        ),
    ] {
        cases.push((id.to_owned(), format!("{foo}{body}")));
    }
    cases.push((
        "an alias's namespace is read where the alias is declared".to_owned(),
        "import React from 'react';\ninterface Props { label?: string }\nconst createElement = React.createElement;\n\
         export const Button = (props: Props) => { const React = {}; return createElement('button', null, props.label); };"
            .to_owned(),
    ));
    let base = "import React from 'react';\ninterface Props { label?: string }\n\
                function Base(props: Props) { return <div />; }\n";
    for (id, body) in [
        (
            "finding: a builder aliased under a withX name",
            "const withElement = React.createElement;\nexport const Button = withElement(Base);",
        ),
        (
            "finding: a parameter shadowing an alias of the builder",
            "const ce = React.createElement;\nexport const Button = (props: Props, ce: (t: string) => string) => ce('button');",
        ),
        (
            "finding: a var in a block, used after it",
            "export const Button = (props: Props) => { { var createElement = (t: string) => t; } return createElement('button'); };",
        ),
        // OXC 0.158 moved these type arguments onto the call, so main read the callee as written plainly.
        ("an instantiation in parentheses", "export const Button = (React.createElement<Props>)(Base);"),
        // Round 18: a call given a tag and a config object, to a factory whose declared result is the component.
        (
            "tagged factory, no props type names the tag | rendered | plain",
            "function make<T>(tag: T, config: object): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = make('button', {});",
        ),
        (
            "tagged factory, no props type names the tag | rendered | typeof itself",
            "function make<T>(tag: T, config: object): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = (make as typeof make)('button', {});",
        ),
        (
            "tagged factory, no props type names the tag | rendered | any",
            "function make<T>(tag: T, config: object): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = (make as any)('button', {});",
        ),
        (
            "tagged factory, a props type names the tag | rendered | plain",
            "interface ButtonProps extends React.ComponentProps<'button'> {}\n\
             function make<T>(tag: T, config: object): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = make('button', {});",
        ),
        // The same slip in the generic wrapper rule: its type arguments name no props type, its callee declares one.
        (
            "generic wrapper, no type argument names props | rendered | plain",
            "function makeX<E, T>(tag: string): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = makeX<HTMLDivElement, Other>('div');",
        ),
        (
            "generic wrapper, no type argument names props | rendered | typeof itself",
            "function makeX<E, T>(tag: string): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = (makeX as typeof makeX)<HTMLDivElement, Other>('div');",
        ),
        // Round 19: a payload whose `props` member is no named type; a wrapper call before the component it wraps; an
        // anonymous default export of a wrapper call.
        (
            "unusable payload | rendered | plain",
            "interface Box<T> { payload?: T }\ntype Payload = Box<{ props: { label: string } }>;\n\
             function makeX<T>(tag: string): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = makeX<Payload>('div');",
        ),
        (
            "unusable payload | rendered | typeof itself",
            "interface Box<T> { payload?: T }\ntype Payload = Box<{ props: { label: string } }>;\n\
             function makeX<T>(tag: string): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = (makeX as typeof makeX)<Payload>('div');",
        ),
        (
            "wrapper call before its component | rendered | plain",
            "function withThing<T>(c: unknown): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = withThing<unknown>(Base2);\nfunction Base2(props: Props) { return <div />; }",
        ),
        (
            "wrapper call before its component | rendered | typeof itself",
            "function withThing<T>(c: unknown): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = (withThing as typeof withThing)<unknown>(Base2);\n\
             function Base2(props: Props) { return <div />; }",
        ),
        (
            "anonymous default wrapper call | rendered | plain",
            "function makeX<T>(tag: string): React.FC<Props> { return (props) => <div />; }\n\
             export default makeX<unknown>('div');",
        ),
        (
            "anonymous default wrapper call | rendered | typeof itself",
            "function makeX<T>(tag: string): React.FC<Props> { return (props) => <div />; }\n\
             export default (makeX as typeof makeX)<unknown>('div');",
        ),
        (
            "tagged factory, a props type names the tag | rendered | typeof itself",
            "interface ButtonProps extends React.ComponentProps<'button'> {}\n\
             function make<T>(tag: T, config: object): React.FC<Props> { return (props) => <div />; }\n\
             export const Button = (make as typeof make)('button', {});",
        ),
        // Round 22: a generic wrapper, tagged factory or factory called through a cast that replaces the callee's type
        // extracts exactly what main extracts, the rows of rounds 20 and 21 included. These ids carry no cast suffix,
        // so each row equals main's.
        (
            "replacing cast: a tagged call beside a plain tagged factory",
            "interface RecipeProps extends React.ComponentProps<'button'> {}\n\
             declare function stringify(tag: string, config: object): string;\n\
             function chakra(tag: string, config: object): React.FC<Props> { return (props) => <div />; }\n\
             const Label = (stringify as (tag: string, config: object) => string)('button', {});\n\
             export const Primary = chakra('button', {});",
        ),
        (
            "replacing cast: a generic wrapper beside a tagged factory, same props type",
            "interface GroupProps extends React.ComponentProps<'div'> {}\n\
             function chakra(tag: string, config: object): React.FC<Props> { return (props) => <div />; }\n\
             function withContext<T, P>(c: unknown): React.FC<P> { return (props) => <div />; }\n\
             const StyledGroup = chakra('div', {});\n\
             export const Group = (withContext as <T, P>(c: unknown) => React.FC<P>)<HTMLDivElement, GroupProps>(StyledGroup);",
        ),
        (
            "replacing cast: a generic wrapper beside a tagged factory, own props type",
            "interface StyledProps extends React.ComponentProps<'div'> {}\n\
             function chakra(tag: string, config: object): React.FC<Props> { return (props) => <div />; }\n\
             function withContext<T, P>(c: unknown): React.FC<P> { return (props) => <div />; }\n\
             const Styled = chakra('div', {});\n\
             export const Group = (withContext as <T, P>(c: unknown) => React.FC<P>)<HTMLDivElement, Props>(Styled);",
        ),
        (
            "plain call: default export, no props type",
            "function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             export default withTheme<unknown>(Base);",
        ),
        (
            "replacing cast: default export, no props type",
            "function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             export default (withTheme as <T>(c: typeof Base) => typeof Base)<unknown>(Base);",
        ),
        (
            "plain call: default export, a props type",
            "function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             export default withTheme<Props>(Base);",
        ),
        (
            "replacing cast: default export, a props type",
            "function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             export default (withTheme as <T>(c: typeof Base) => typeof Base)<Props>(Base);",
        ),
        (
            "plain call: named export, no props type",
            "function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             export const Themed = withTheme<unknown>(Base);",
        ),
        (
            "replacing cast: named export, no props type",
            "function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             export const Themed = (withTheme as <T>(c: typeof Base) => typeof Base)<unknown>(Base);",
        ),
        (
            "plain call: named export, a props type",
            "function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             export const Themed = withTheme<Props>(Base);",
        ),
        (
            "replacing cast: named export, a props type",
            "function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             export const Themed = (withTheme as <T>(c: typeof Base) => typeof Base)<Props>(Base);",
        ),
        (
            "plain call: named export, a builder, no props type",
            "function makeX<T>(tag: string): React.FC<Props> { return (props) => <div />; }\n\
             export const Themed = makeX<unknown>('div');",
        ),
        (
            "replacing cast: named export, a builder, no props type",
            "function makeX<T>(tag: string): React.FC<Props> { return (props) => <div />; }\n\
             export const Themed = (makeX as <T>(tag: string) => React.FC<Props>)<unknown>('div');",
        ),
        (
            "replacing cast: an alias and a default export of a generic wrapper",
            "function withContext<T, P>(c: unknown): React.FC<P> { return (props) => <div />; }\n\
             const Group = (withContext as <T, P>(c: unknown) => React.FC<P>)<HTMLDivElement, Props>(Base);\n\
             export const Alias = Group;\nexport default Group;",
        ),
        (
            "replacing cast: a member generic wrapper",
            "interface PublicProps { label?: string }\ndeclare const ui: { withContext<P>(c: unknown): React.FC<P> };\n\
             type SomeType = { withContext<P>(c: unknown): React.FC<P> };\n\
             export const Card = (ui as SomeType).withContext<PublicProps>(Base);",
        ),
        (
            "replacing cast: a generic wrapper to a named signature",
            "interface Public { label?: string }\n\
             function withTheme<T>(c: typeof Base): typeof Base { return c; }\n\
             type Signature = <T>(c: unknown) => React.FC<T>;\n\
             export const Card = (withTheme as Signature)<Public>(Base);",
        ),
        (
            "replacing cast: a factory to a named type",
            "function createButton(): React.FC<Props> { return (props) => <div />; }\n\
             type SomeFactory = () => React.FC<{ tone?: string }>;\n\
             export const Card = (createButton as SomeFactory)();",
        ),
        // Round 26: a callee written through a cast is attached only when the file declares its name once.
        (
            "callee declared once | plain",
            "function wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = wrap<Props>((props: Props) => <div />);",
        ),
        (
            "callee declared once | typeof itself",
            "function wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = (wrap as typeof wrap)<Props>((props: Props) => <div />);",
        ),
        (
            "callee declared twice at module level | plain",
            "function wrap<T>(render: React.FC<T>): React.FC<T>;\nfunction wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = wrap<Props>((props: Props) => <div />);",
        ),
        (
            "callee declared twice at module level | typeof itself",
            "function wrap<T>(render: React.FC<T>): React.FC<T>;\nfunction wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = (wrap as typeof wrap)<Props>((props: Props) => <div />);",
        ),
        (
            "callee declared again in a namespace | plain",
            "namespace N { export function wrap<T>(render: React.FC<T>): React.FC<T> { return render; } }\nfunction wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = wrap<Props>((props: Props) => <div />);",
        ),
        (
            "callee declared again in a namespace | typeof itself",
            "namespace N { export function wrap<T>(render: React.FC<T>): React.FC<T> { return render; } }\nfunction wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = (wrap as typeof wrap)<Props>((props: Props) => <div />);",
        ),
        (
            "callee name a parameter elsewhere | plain",
            "function g(wrap: unknown) { return wrap; }\nfunction wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = wrap<Props>((props: Props) => <div />);",
        ),
        (
            "callee name a parameter elsewhere | typeof itself",
            "function g(wrap: unknown) { return wrap; }\nfunction wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = (wrap as typeof wrap)<Props>((props: Props) => <div />);",
        ),
        (
            "callee imported and declared | plain",
            "import { wrap } from './wrap';\nfunction wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = wrap<Props>((props: Props) => <div />);",
        ),
        (
            "callee imported and declared | typeof itself",
            "import { wrap } from './wrap';\nfunction wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\nexport const Button = (wrap as typeof wrap)<Props>((props: Props) => <div />);",
        ),
        (
            "finding: a namespace's own wrap called through typeof itself",
            "function wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\n\
             namespace N {\n\
             function wrap<T>(render: React.FC<T>): React.FC<T> { return render; }\n\
             export const Inner = (wrap as typeof wrap)<Props>((props: Props) => <div />);\n\
             }",
        ),
        // Round 27: an Object.assign written through a cast attaches its members only at module level and only when
        // the file declares the parent once.
        (
            "Object.assign a shadowing local Root in a function | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction build() { function Root(props: Props) { return <span />; } Object.assign(Root, { Item: (p: ItemProps) => <div /> }); }",
        ),
        (
            "Object.assign a shadowing local Root in a function | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction build() { function Root(props: Props) { return <span />; } (Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> }); }",
        ),
        (
            "Object.assign at module level | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign at module level | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign in a nested function | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction build() { function inner() { Object.assign(Root, { Item: (p: ItemProps) => <div /> }); } return inner; }",
        ),
        (
            "Object.assign in a nested function | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction build() { function inner() { (Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> }); } return inner; }",
        ),
        (
            "Object.assign in an arrow initializer | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nexport const build = () => Object.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign in an arrow initializer | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nexport const build = () => (Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign in a class method | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nclass K { m() { Object.assign(Root, { Item: (p: ItemProps) => <div /> }); } }",
        ),
        (
            "Object.assign in a class method | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nclass K { m() { (Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> }); } }",
        ),
        (
            "Object.assign as the default export | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nexport default Object.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign as the default export | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nexport default (Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign as an exported declaration's initializer | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nexport const Compound = Object.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign as an exported declaration's initializer | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nexport const Compound = (Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign with a second module-level Root | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nnamespace N { export const Root = 1; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign with a second module-level Root | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nnamespace N { export const Root = 1; }\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        // Round 28: a receiver read through a cast; static metadata assigned through a cast.
        (
            "member callee receiver unshadowed | plain",
            "import * as ui from './ui';\nexport const Themed = ui.wrap<Props>((props: Props) => <div />);",
        ),
        (
            "member callee receiver unshadowed | typeof itself",
            "import * as ui from './ui';\nexport const Themed = (ui as typeof ui).wrap<Props>((props: Props) => <div />);",
        ),
        (
            "member callee receiver shadowed by a namespace local | plain",
            "import * as ui from './ui';\nnamespace N { const ui = { wrap<T>(r: React.FC<T>): React.FC<T> { return r; } }; export const Themed = ui.wrap<Props>((props: Props) => <div />); }",
        ),
        (
            "member callee receiver shadowed by a namespace local | typeof itself",
            "import * as ui from './ui';\nnamespace N { const ui = { wrap<T>(r: React.FC<T>): React.FC<T> { return r; } }; export const Themed = (ui as typeof ui).wrap<Props>((props: Props) => <div />); }",
        ),
        (
            "static defaultProps unshadowed | plain",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nButton.defaultProps = { label: 'local' };",
        ),
        (
            "static defaultProps unshadowed | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\n(Button as typeof Button).defaultProps = { label: 'local' };",
        ),
        (
            "static defaultProps shadowed by a parameter | plain",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nexport function configure(Button: unknown) { Button.defaultProps = { label: 'local' }; }",
        ),
        (
            "static defaultProps shadowed by a parameter | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nexport function configure(Button: unknown) { (Button as typeof Button).defaultProps = { label: 'local' }; }",
        ),
        (
            "static displayName unshadowed | plain",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nButton.displayName = 'Renamed';",
        ),
        (
            "static displayName unshadowed | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\n(Button as typeof Button).displayName = 'Renamed';",
        ),
        (
            "static displayName shadowed by a parameter | plain",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nexport function configure(Button: unknown) { Button.displayName = 'Renamed'; }",
        ),
        (
            "static displayName shadowed by a parameter | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nexport function configure(Button: unknown) { (Button as typeof Button).displayName = 'Renamed'; }",
        ),
        (
            "static a member set to a component unshadowed | plain",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nButton.Item = Item;",
        ),
        (
            "static a member set to a component unshadowed | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\n(Button as typeof Button).Item = Item;",
        ),
        (
            "static a member set to a component shadowed by a parameter | plain",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nexport function configure(Button: unknown) { Button.Item = Item; }",
        ),
        (
            "static a member set to a component shadowed by a parameter | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nexport function configure(Button: unknown) { (Button as typeof Button).Item = Item; }",
        ),
        (
            "static a member set to an inline component unshadowed | plain",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nButton.Inline = (p: ItemProps) => <div />;",
        ),
        (
            "static a member set to an inline component unshadowed | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\n(Button as typeof Button).Inline = (p: ItemProps) => <div />;",
        ),
        (
            "static a member set to an inline component shadowed by a parameter | plain",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nexport function configure(Button: unknown) { Button.Inline = (p: ItemProps) => <div />; }",
        ),
        (
            "static a member set to an inline component shadowed by a parameter | typeof itself",
            "interface ItemProps { tone?: string }\nexport function Button(props: Props) { return <div />; }\nexport function Item(props: ItemProps) { return <div />; }\nexport function configure(Button: unknown) { (Button as typeof Button).Inline = (p: ItemProps) => <div />; }",
        ),
        // Round 29: every receiver read through a wrapper goes through `receiver_identifier`.
        (
            "receiver of a alias, declared once | plain",
            "import * as Ui from './ui';\nexport const PublicButton = Ui.Button;",
        ),
        (
            "receiver of a alias, declared once | typeof itself",
            "import * as Ui from './ui';\nexport const PublicButton = (Ui as typeof Ui).Button;",
        ),
        (
            "receiver of a alias, shadowed by a namespace local | plain",
            "import * as Ui from './ui';\nnamespace N { const Ui = { Button: 123 }; }\nexport const PublicButton = Ui.Button;",
        ),
        (
            "receiver of a alias, shadowed by a namespace local | typeof itself",
            "import * as Ui from './ui';\nnamespace N { const Ui = { Button: 123 }; }\nexport const PublicButton = (Ui as typeof Ui).Button;",
        ),
        (
            "receiver of a alias, shadowed by a parameter | plain",
            "import * as Ui from './ui';\nfunction f(Ui: unknown) { return Ui; }\nexport const PublicButton = Ui.Button;",
        ),
        (
            "receiver of a alias, shadowed by a parameter | typeof itself",
            "import * as Ui from './ui';\nfunction f(Ui: unknown) { return Ui; }\nexport const PublicButton = (Ui as typeof Ui).Button;",
        ),
        (
            "receiver of a alias, imported and declared | plain",
            "import * as Ui from './ui';\nconst Ui = { Button: 123 };\nexport const PublicButton = Ui.Button;",
        ),
        (
            "receiver of a alias, imported and declared | typeof itself",
            "import * as Ui from './ui';\nconst Ui = { Button: 123 };\nexport const PublicButton = (Ui as typeof Ui).Button;",
        ),
        (
            "receiver of a callee, declared once | plain",
            "import * as Ui from './ui';\nexport const Themed = Ui.wrap<Props>((props: Props) => <div />);",
        ),
        (
            "receiver of a callee, declared once | typeof itself",
            "import * as Ui from './ui';\nexport const Themed = (Ui as typeof Ui).wrap<Props>((props: Props) => <div />);",
        ),
        (
            "receiver of a callee, shadowed by a namespace local | plain",
            "import * as Ui from './ui';\nnamespace N { const Ui = { Button: 123 }; }\nexport const Themed = Ui.wrap<Props>((props: Props) => <div />);",
        ),
        (
            "receiver of a callee, shadowed by a namespace local | typeof itself",
            "import * as Ui from './ui';\nnamespace N { const Ui = { Button: 123 }; }\nexport const Themed = (Ui as typeof Ui).wrap<Props>((props: Props) => <div />);",
        ),
        (
            "receiver of a callee, shadowed by a parameter | plain",
            "import * as Ui from './ui';\nfunction f(Ui: unknown) { return Ui; }\nexport const Themed = Ui.wrap<Props>((props: Props) => <div />);",
        ),
        (
            "receiver of a callee, shadowed by a parameter | typeof itself",
            "import * as Ui from './ui';\nfunction f(Ui: unknown) { return Ui; }\nexport const Themed = (Ui as typeof Ui).wrap<Props>((props: Props) => <div />);",
        ),
        (
            "receiver of a callee, imported and declared | plain",
            "import * as Ui from './ui';\nconst Ui = { Button: 123 };\nexport const Themed = Ui.wrap<Props>((props: Props) => <div />);",
        ),
        (
            "receiver of a callee, imported and declared | typeof itself",
            "import * as Ui from './ui';\nconst Ui = { Button: 123 };\nexport const Themed = (Ui as typeof Ui).wrap<Props>((props: Props) => <div />);",
        ),
        (
            "receiver of a heritage, declared once | plain",
            "import * as Ui from 'react';\nexport class Card extends Ui.Component<Props> { render() { return <div />; } }",
        ),
        (
            "receiver of a heritage, declared once | typeof itself",
            "import * as Ui from 'react';\nexport class Card extends (Ui as typeof Ui).Component<Props> { render() { return <div />; } }",
        ),
        (
            "receiver of a heritage, shadowed by a namespace local | plain",
            "import * as Ui from 'react';\nnamespace N { const Ui = { Button: 123 }; }\nexport class Card extends Ui.Component<Props> { render() { return <div />; } }",
        ),
        (
            "receiver of a heritage, shadowed by a namespace local | typeof itself",
            "import * as Ui from 'react';\nnamespace N { const Ui = { Button: 123 }; }\nexport class Card extends (Ui as typeof Ui).Component<Props> { render() { return <div />; } }",
        ),
        (
            "receiver of a heritage, shadowed by a parameter | plain",
            "import * as Ui from 'react';\nfunction f(Ui: unknown) { return Ui; }\nexport class Card extends Ui.Component<Props> { render() { return <div />; } }",
        ),
        (
            "receiver of a heritage, shadowed by a parameter | typeof itself",
            "import * as Ui from 'react';\nfunction f(Ui: unknown) { return Ui; }\nexport class Card extends (Ui as typeof Ui).Component<Props> { render() { return <div />; } }",
        ),
        (
            "receiver of a heritage, imported and declared | plain",
            "import * as Ui from 'react';\nconst Ui = { Button: 123 };\nexport class Card extends Ui.Component<Props> { render() { return <div />; } }",
        ),
        (
            "receiver of a heritage, imported and declared | typeof itself",
            "import * as Ui from 'react';\nconst Ui = { Button: 123 };\nexport class Card extends (Ui as typeof Ui).Component<Props> { render() { return <div />; } }",
        ),
        (
            "finding: a namespace-local receiver read through a cast",
            "import * as Ui from './ui';\nnamespace N { const Ui = { Button: 123 }; export const PublicButton = (Ui as typeof Ui).Button; }",
        ),
        // Round 30: Recognition. Each pair is written plainly and in a shape only a peel makes readable.
        (
            "recognition: heritage, a local class (finding 1) | plain",
            "class Component<P> {}\nexport class Record extends Component<Props> { render() { return <div />; } }",
        ),
        (
            "recognition: heritage, a local class (finding 1) | peeled",
            "class Component<P> {}\nexport class Record extends (Component as typeof Component)<Props> { render() { return <div />; } }",
        ),
        (
            "recognition: heritage, an import declared once | plain",
            "import { Component } from 'react';\nexport class Record extends Component<Props> { render() { return <div />; } }",
        ),
        (
            "recognition: heritage, an import declared once | peeled",
            "import { Component } from 'react';\nexport class Record extends (Component as typeof Component)<Props> { render() { return <div />; } }",
        ),
        (
            "recognition: heritage, a global | plain",
            "export class Record extends Component<Props> { render() { return <div />; } }",
        ),
        (
            "recognition: heritage, a global | peeled",
            "export class Record extends (Component as typeof Component)<Props> { render() { return <div />; } }",
        ),
        (
            "recognition: Object.assign member, declared twice (finding 2) | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction Item(props: ItemProps) { return <div />; }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item });",
        ),
        (
            "recognition: Object.assign member, declared twice (finding 2) | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction Item(props: ItemProps) { return <div />; }\nnamespace N { const Item = 123; }\n(Object.assign as typeof Object.assign)(Root, { Item });",
        ),
        (
            "recognition: Object.assign member, declared once | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction Item(props: ItemProps) { return <div />; }\nObject.assign(Root, { Item });",
        ),
        (
            "recognition: Object.assign member, declared once | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction Item(props: ItemProps) { return <div />; }\n(Object.assign as typeof Object.assign)(Root, { Item });",
        ),
        (
            "recognition: Object.assign object behind as const, a shadowing local (finding 3) | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction build() { function Root(props: Props) { return <span />; } Object.assign(Root, { Item: (p: ItemProps) => <div /> }); }",
        ),
        (
            "recognition: Object.assign object behind as const, a shadowing local (finding 3) | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction build() { function Root(props: Props) { return <span />; } Object.assign(Root, { Item: (p: ItemProps) => <div /> } as const); }",
        ),
        (
            "recognition: Object.assign object behind as const, at module level | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "recognition: Object.assign object behind as const, at module level | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> } as const);",
        ),
        (
            "recognition: Object.assign object behind satisfies, a shadowing local | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction build() { function Root(props: Props) { return <span />; } Object.assign(Root, { Item: (p: ItemProps) => <div /> }); }",
        ),
        (
            "recognition: Object.assign object behind satisfies, a shadowing local | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction build() { function Root(props: Props) { return <span />; } Object.assign(Root, { Item: (p: ItemProps) => <div /> } satisfies object); }",
        ),
        (
            "recognition: factory callee behind an instantiation, declared once | plain",
            "function createButton<T>(): React.FC<Props> { return (props) => <div />; }\nexport const Card = createButton();",
        ),
        (
            "recognition: factory callee behind an instantiation, declared once | peeled",
            "function createButton<T>(): React.FC<Props> { return (props) => <div />; }\nexport const Card = (createButton<unknown>)();",
        ),
        (
            "recognition: factory callee behind an instantiation, a parameter of its name | plain",
            "function createButton<T>(): React.FC<Props> { return (props) => <div />; }\nfunction g(createButton: unknown) { return createButton; }\nexport const Card = createButton();",
        ),
        (
            "recognition: factory callee behind an instantiation, a parameter of its name | peeled",
            "function createButton<T>(): React.FC<Props> { return (props) => <div />; }\nfunction g(createButton: unknown) { return createButton; }\nexport const Card = (createButton<unknown>)();",
        ),
        // Round 31: a well-known global or import a peeled path relies on.
        (
            "well-known Preact h, an import declared once | plain",
            "import * as Preact from 'preact';\nexport const Button = (props: Props) => Preact.h('button');",
        ),
        (
            "well-known Preact h, an import declared once | peeled",
            "import * as Preact from 'preact';\nexport const Button = (props: Props) => (Preact as typeof Preact).h('button');",
        ),
        (
            "well-known Preact h, a parameter of its name (finding 1) | plain",
            "import * as Preact from 'preact';\nexport const Button = (Preact: Props) => Preact.h('button');",
        ),
        (
            "well-known Preact h, a parameter of its name (finding 1) | peeled",
            "import * as Preact from 'preact';\nexport const Button = (Preact: Props) => (Preact as typeof Preact).h('button');",
        ),
        (
            "well-known Preact h, a namespace local of its name | plain",
            "import * as Preact from 'preact';\nnamespace N { const Preact = 1; }\nexport const Button = (props: Props) => Preact.h('button');",
        ),
        (
            "well-known Preact h, a namespace local of its name | peeled",
            "import * as Preact from 'preact';\nnamespace N { const Preact = 1; }\nexport const Button = (props: Props) => (Preact as typeof Preact).h('button');",
        ),
        (
            "well-known React createElement, an import declared once | plain",
            "import * as R from 'react';\nexport const Button = (props: Props) => R.createElement('button');",
        ),
        (
            "well-known React createElement, an import declared once | peeled",
            "import * as R from 'react';\nexport const Button = (props: Props) => (R as typeof R).createElement('button');",
        ),
        (
            "well-known React createElement, a parameter of its name | plain",
            "import * as R from 'react';\nexport const Button = (R: Props) => R.createElement('button');",
        ),
        (
            "well-known React createElement, a parameter of its name | peeled",
            "import * as R from 'react';\nexport const Button = (R: Props) => (R as typeof R).createElement('button');",
        ),
        (
            "well-known React createElement, a namespace local of its name | plain",
            "import * as R from 'react';\nnamespace N { const R = 1; }\nexport const Button = (props: Props) => R.createElement('button');",
        ),
        (
            "well-known React createElement, a namespace local of its name | peeled",
            "import * as R from 'react';\nnamespace N { const R = 1; }\nexport const Button = (props: Props) => (R as typeof R).createElement('button');",
        ),
        ("well-known Promise, the global | plain", "export const Button = (props: Props) => Promise.resolve(<div />);"),
        (
            "well-known Promise, the global | peeled",
            "export const Button = (props: Props) => (Promise as typeof Promise).resolve(<div />);",
        ),
        (
            "well-known Promise, a local of its name | plain",
            "const Promise = { resolve: (x: unknown) => 1 };\nexport const Button = (props: Props) => Promise.resolve(<div />);",
        ),
        (
            "well-known Promise, a local of its name | peeled",
            "const Promise = { resolve: (x: unknown) => 1 };\nexport const Button = (props: Props) => (Promise as typeof Promise).resolve(<div />);",
        ),
        (
            "well-known Object.assign, the global | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "well-known Object.assign, the global | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "well-known Object.assign, a local Object (finding 2) | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nconst Object = { assign: (root: unknown, members: unknown) => root };\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "well-known Object.assign, a local Object (finding 2) | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nconst Object = { assign: (root: unknown, members: unknown) => root };\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "well-known Object.assign, a parameter of its name | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction f(Object: unknown) { return Object; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "well-known Object.assign, a parameter of its name | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nfunction f(Object: unknown) { return Object; }\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "well-known Object.assign, assigned in the file | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign = ((root: unknown) => root) as typeof Object.assign;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "well-known Object.assign, assigned in the file | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign = ((root: unknown) => root) as typeof Object.assign;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        // Round 32: what an `Object.assign` call's arguments are, as `assign_roles` reads them.
        (
            "Object.assign roles, a source declared once (finding 1) | plain",
            "interface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return <div />; }\nexport const Widget = Object.assign({}, Item);",
        ),
        (
            "Object.assign roles, a source declared once (finding 1) | peeled",
            "interface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return <div />; }\nexport const Widget = (Object.assign as typeof Object.assign)({}, Item);",
        ),
        (
            "Object.assign roles, a source declared twice | plain",
            "interface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return <div />; }\nnamespace N { const Item = 1; }\nexport const Widget = Object.assign({}, Item);",
        ),
        (
            "Object.assign roles, a source declared twice | peeled",
            "interface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return <div />; }\nnamespace N { const Item = 1; }\nexport const Widget = (Object.assign as typeof Object.assign)({}, Item);",
        ),
        (
            "Object.assign roles, an imported source | plain",
            "import { Item } from './item';\nexport const Widget = Object.assign({}, Item);",
        ),
        (
            "Object.assign roles, an imported source | peeled",
            "import { Item } from './item';\nexport const Widget = (Object.assign as typeof Object.assign)({}, Item);",
        ),
        (
            "Object.assign roles, a target declared once | plain",
            "export function Root(props: Props) { return <div />; }\nexport const Compound = Object.assign(Root, {});",
        ),
        (
            "Object.assign roles, a target declared once | peeled",
            "export function Root(props: Props) { return <div />; }\nexport const Compound = (Object.assign as typeof Object.assign)(Root, {});",
        ),
        (
            "Object.assign roles, a target declared twice (finding 2) | plain",
            "export function Root(props: Props) { return <div />; }\nnamespace N { const Root = {}; export const Compound = Object.assign(Root, {}); }",
        ),
        (
            "Object.assign roles, a target declared twice (finding 2) | peeled",
            "export function Root(props: Props) { return <div />; }\nnamespace N { const Root = {}; export const Compound = (Object.assign as typeof Object.assign)(Root, {}); }",
        ),
        (
            "Object.assign roles, a private target declared twice, bound at module level | plain",
            "function Root(props: Props) { return <div />; }\nnamespace N { const Root = 1; }\nexport const Compound = Object.assign(Root, {});",
        ),
        (
            "Object.assign roles, a private target declared twice, bound at module level | peeled",
            "function Root(props: Props) { return <div />; }\nnamespace N { const Root = 1; }\nexport const Compound = (Object.assign as typeof Object.assign)(Root, {});",
        ),
        (
            "Object.assign roles, an imported target | plain",
            "import { Root } from './root';\nexport const Compound = Object.assign(Root, {});",
        ),
        (
            "Object.assign roles, an imported target | peeled",
            "import { Root } from './root';\nexport const Compound = (Object.assign as typeof Object.assign)(Root, {});",
        ),
        (
            "Object.assign roles, a default export of a target declared once | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nexport default Object.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign roles, a default export of a target declared once | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nexport default (Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign roles, a default export of a target declared twice | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nnamespace N { const Root = 1; }\nexport default Object.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "Object.assign roles, a default export of a target declared twice | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nnamespace N { const Root = 1; }\nexport default (Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        // Round 33: a global's name imported or shadowed, behind a cast.
        (
            "well-known table: Promise, an import of its name (finding 2) | plain",
            "import Promise from './codec';\nexport const Button = (props: Props) => Promise.resolve(<div />);",
        ),
        (
            "well-known table: Promise, an import of its name (finding 2) | peeled",
            "import Promise from './codec';\nexport const Button = (props: Props) => (Promise as typeof Promise).resolve(<div />);",
        ),
        (
            "well-known table: Promise, a parameter of its name | plain",
            "function g(Promise: unknown) { return Promise; }\nexport const Button = (props: Props) => Promise.resolve(<div />);",
        ),
        (
            "well-known table: Promise, a parameter of its name | peeled",
            "function g(Promise: unknown) { return Promise; }\nexport const Button = (props: Props) => (Promise as typeof Promise).resolve(<div />);",
        ),
        // Round 34: the minimal shape and the writes a peeled `Object.assign` is recognised against.
        (
            "UniqueName: a later source overwrites the member (finding 1) | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> }, { Item: null });",
        ),
        (
            "UniqueName: a later source overwrites the member (finding 1) | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> }, { Item: null });",
        ),
        (
            "UniqueName: a spread member | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nconst extra = {};\nObject.assign(Root, { ...extra, Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: a spread member | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nconst extra = {};\n(Object.assign as typeof Object.assign)(Root, { ...extra, Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: a computed key | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { ['Item']: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: a computed key | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.assign as typeof Object.assign)(Root, { ['Item']: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: a repeated key | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { Item: (p: ItemProps) => <div />, Item: null });",
        ),
        (
            "UniqueName: a repeated key | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div />, Item: null });",
        ),
        (
            "UniqueName: Object assign written through a string key (finding 2) | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['assign'] = ((root: unknown) => root) as typeof Object.assign;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through a string key (finding 2) | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['assign'] = ((root: unknown) => root) as typeof Object.assign;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through a template key | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject[`assign`] = ((root: unknown) => root) as typeof Object.assign;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through a template key | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject[`assign`] = ((root: unknown) => root) as typeof Object.assign;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through Reflect.set | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nReflect.set(Object, 'assign', (root: unknown) => root);\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through Reflect.set | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nReflect.set(Object, 'assign', (root: unknown) => root);\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through defineProperty | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.defineProperty(Object, 'assign', { value: (root: unknown) => root });\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through defineProperty | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.defineProperty(Object, 'assign', { value: (root: unknown) => root });\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through defineProperties | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.defineProperties(Object, { assign: { value: 1 } });\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object assign written through defineProperties | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.defineProperties(Object, { assign: { value: 1 } });\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object written through an unknown key | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const k: string;\nObject[k] = 1;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: Object written through an unknown key | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const k: string;\nObject[k] = 1;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: another Object member written | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['keys'] = Object.keys;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "UniqueName: another Object member written | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['keys'] = Object.keys;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        // Round 36: names a sealed peel reads, behind a cast.
        (
            "sealed: a wrapped mutator writes Object assign (finding 2) | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.defineProperty as typeof Object.defineProperty)(Object, 'assign', { value: (target: unknown) => target });\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "sealed: a wrapped mutator writes Object assign (finding 2) | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.defineProperty as typeof Object.defineProperty)(Object, 'assign', { value: (target: unknown) => target });\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "sealed: a new prototype for Object | plain",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.setPrototypeOf(Object, null);\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "sealed: a new prototype for Object | peeled",
            "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.setPrototypeOf(Object, null);\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });",
        ),
        (
            "sealed: a factory on another module's receiver (finding 5) | plain",
            "import * as codec from './codec';\nexport const Button = (props: Props) => codec.createElement('button');",
        ),
        (
            "sealed: a factory on another module's receiver (finding 5) | peeled",
            "import * as codec from './codec';\nexport const Button = (props: Props) => (codec.createElement as typeof codec.createElement)('button');",
        ),
        (
            "sealed: a factory on React's receiver | plain",
            "import * as R from 'react';\nexport const Button = (props: Props) => R.createElement('button');",
        ),
        (
            "sealed: a factory on React's receiver | peeled",
            "import * as R from 'react';\nexport const Button = (props: Props) => (R.createElement as typeof R.createElement)('button');",
        ),
        // Round 37: evidence predicates on a callee read through a cast.
        (
            "evidence: a bare local map (finding 1) | plain",
            "function map(cb: unknown) { return new Response(); }\nexport const Button = (props: Props) => map(() => <div />);",
        ),
        (
            "evidence: a bare local map (finding 1) | peeled",
            "function map(cb: unknown) { return new Response(); }\nexport const Button = (props: Props) => (map as typeof map)(() => <div />);",
        ),
        (
            "evidence: a map on a value of unknown type | plain",
            "declare const items: number[];\nexport const Button = (props: Props) => items.map(() => <div />);",
        ),
        (
            "evidence: a map on a value of unknown type | peeled",
            "declare const items: number[];\nexport const Button = (props: Props) => (items.map as typeof items.map)(() => <div />);",
        ),
        (
            "evidence: a map on an array literal | plain",
            "export const Button = (props: Props) => [1].map(() => <div />);",
        ),
        (
            "evidence: a map on an array literal | peeled",
            "export const Button = (props: Props) => ([1].map as (f: unknown) => unknown)(() => <div />);",
        ),
        (
            "evidence: a helper shadowed by a destructured prop (finding 2) | plain",
            "function render(): React.ReactElement { return <div />; }\ninterface RenderProps { render: () => symbol }\nexport const Button = ({ render }: RenderProps) => render();",
        ),
        (
            "evidence: a helper shadowed by a destructured prop (finding 2) | peeled",
            "function render(): React.ReactElement { return <div />; }\ninterface RenderProps { render: () => symbol }\nexport const Button = ({ render }: RenderProps) => (render as typeof render)();",
        ),
        (
            "evidence: a helper declared once | plain",
            "function render(): React.ReactElement { return <div />; }\nexport const Button = (props: Props) => render();",
        ),
        (
            "evidence: a helper declared once | peeled",
            "function render(): React.ReactElement { return <div />; }\nexport const Button = (props: Props) => (render as typeof render)();",
        ),
        (
            "finding: a require's member",
            "export const Button = (props: Props) => require('react').createElement('button');",
        ),
        (
            "finding: a named import's member from a relative module",
            "import { toolkit } from './widgets';\nexport const Button = toolkit.createElement<Props>(Base);",
        ),
    ] {
        cases.push((id.to_owned(), format!("{base}{body}")));
    }
    for (id, inner) in [
        ("an element built from forwardRef, plain", "React.forwardRef<HTMLElement, Props>((props, ref) => <div />)"),
        ("an element built from forwardRef, aliased", "fr<HTMLElement, Props>((props, ref) => <div />)"),
        ("an element built from memo, aliased", "m((props: Props) => <div />)"),
    ] {
        cases.push((
            id.to_owned(),
            format!(
                "import React from 'react';\ninterface Props {{ label?: string }}\nconst fr = React.forwardRef;\n\
                 const m = React.memo;\nexport const Button = React.createElement({inner}, null);"
            ),
        ));
    }
    // Round 32: `react_api` behind a `<T>x` assertion, a `.ts` file's form.
    cases.push((
        "react_api behind a type assertion, an import declared once | plain".to_owned(),
        "// ts\nimport * as React from 'react';\ninterface Props { label?: string }\nexport const Button = React.forwardRef<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "react_api behind a type assertion, an import declared once | peeled".to_owned(),
        "// ts\nimport * as React from 'react';\ninterface Props { label?: string }\nexport const Button = (<typeof React>React).forwardRef<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "react_api behind a type assertion, a parameter of its name | plain".to_owned(),
        "// ts\nimport * as React from 'react';\ninterface Props { label?: string }\nfunction f(React: unknown) { return React; }\nexport const Button = React.forwardRef<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "react_api behind a type assertion, a parameter of its name | peeled".to_owned(),
        "// ts\nimport * as React from 'react';\ninterface Props { label?: string }\nfunction f(React: unknown) { return React; }\nexport const Button = (<typeof React>React).forwardRef<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    // Round 33: each well-known entry behind a `<T>x` assertion, a `.ts` file's form.
    cases.push((
        "well-known table: forwardRef, an import declared once | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { forwardRef } from 'react';\nexport const Button = forwardRef<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: forwardRef, an import declared once | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { forwardRef } from 'react';\nexport const Button = (<typeof forwardRef>forwardRef)<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: forwardRef, an aliased import | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { forwardRef as fr } from 'react';\nexport const Button = fr<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: forwardRef, an aliased import | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { forwardRef as fr } from 'react';\nexport const Button = (<typeof fr>fr)<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: forwardRef, an aliased import shadowed by a namespace local (finding 1) | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { forwardRef as fr } from 'react';\nnamespace N { export function fr<E, P>(tag: string): symbol { return Symbol(); } }\nexport const Button = fr<HTMLButtonElement, Props>('button');".to_owned(),
    ));
    cases.push((
        "well-known table: forwardRef, an aliased import shadowed by a namespace local (finding 1) | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { forwardRef as fr } from 'react';\nnamespace N { export function fr<E, P>(tag: string): symbol { return Symbol(); } }\nexport const Button = (<typeof fr>fr)<HTMLButtonElement, Props>('button');".to_owned(),
    ));
    cases.push((
        "well-known table: forwardRef, an aliased import shadowed by a parameter | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { forwardRef as fr } from 'react';\nfunction g(fr: unknown) { return fr; }\nexport const Button = fr<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: forwardRef, an aliased import shadowed by a parameter | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { forwardRef as fr } from 'react';\nfunction g(fr: unknown) { return fr; }\nexport const Button = (<typeof fr>fr)<HTMLElement, Props>((props, ref) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: memo, an import declared once | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { memo } from 'react';\nexport const Button = memo((props: Props) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: memo, an import declared once | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { memo } from 'react';\nexport const Button = (<typeof memo>memo)((props: Props) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: memo, an import shadowed by a parameter | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { memo } from 'react';\nfunction g(memo: unknown) { return memo; }\nexport const Button = memo((props: Props) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: memo, an import shadowed by a parameter | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { memo } from 'react';\nfunction g(memo: unknown) { return memo; }\nexport const Button = (<typeof memo>memo)((props: Props) => null);".to_owned(),
    ));
    cases.push((
        "well-known table: createElement, an import declared once | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { createElement } from 'react';\nexport const Button = (props: Props) => createElement('button');".to_owned(),
    ));
    cases.push((
        "well-known table: createElement, an import declared once | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { createElement } from 'react';\nexport const Button = (props: Props) => (<typeof createElement>createElement)('button');".to_owned(),
    ));
    cases.push((
        "well-known table: createElement, an import shadowed by a parameter | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { createElement } from 'react';\nfunction g(createElement: unknown) { return createElement; }\nexport const Button = (props: Props) => createElement('button');".to_owned(),
    ));
    cases.push((
        "well-known table: createElement, an import shadowed by a parameter | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { createElement } from 'react';\nfunction g(createElement: unknown) { return createElement; }\nexport const Button = (props: Props) => (<typeof createElement>createElement)('button');".to_owned(),
    ));
    cases.push((
        "well-known table: h, an import declared once | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { h } from 'preact';\nexport const Button = (props: Props) => h('button');".to_owned(),
    ));
    cases.push((
        "well-known table: h, an import declared once | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { h } from 'preact';\nexport const Button = (props: Props) => (<typeof h>h)('button');".to_owned(),
    ));
    cases.push((
        "well-known table: h, an import shadowed by a parameter | plain".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { h } from 'preact';\nfunction g(h: unknown) { return h; }\nexport const Button = (props: Props) => h('button');".to_owned(),
    ));
    cases.push((
        "well-known table: h, an import shadowed by a parameter | peeled".to_owned(),
        "// ts\ninterface Props { label?: string }\nimport { h } from 'preact';\nfunction g(h: unknown) { return h; }\nexport const Button = (props: Props) => (<typeof h>h)('button');".to_owned(),
    ));
    // Round 34: an identifier alias behind a `<T>x` assertion.
    cases.push((
        "UniqueName: an identifier alias declared once | plain".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\nexport function Root(props: Props) { return null; }\nexport const PublicRoot = Root;".to_owned(),
    ));
    cases.push((
        "UniqueName: an identifier alias declared once | peeled".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\nexport function Root(props: Props) { return null; }\nexport const PublicRoot = <typeof Root>Root;".to_owned(),
    ));
    cases.push((
        "UniqueName: an identifier alias shadowed by a namespace local (finding 3) | plain".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\nexport function Root(props: Props) { return null; }\nnamespace N { const Root = 123; export const PublicRoot = Root; }".to_owned(),
    ));
    cases.push((
        "UniqueName: an identifier alias shadowed by a namespace local (finding 3) | peeled".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\nexport function Root(props: Props) { return null; }\nnamespace N { const Root = 123; export const PublicRoot = <typeof Root>Root; }".to_owned(),
    ));
    // Round 36: names a sealed peel reads, behind a `<T>x` assertion.
    cases.push((
        "sealed: a member assigned through an assertion, a parameter of its name (finding 1) | plain".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\ninterface ItemProps { tone?: string }\nexport function Root(props: Props) { return null; }\nfunction Item(props: ItemProps) { return null; }\nfunction configure(Item: number) { Root.Item = Item; }".to_owned(),
    ));
    cases.push((
        "sealed: a member assigned through an assertion, a parameter of its name (finding 1) | peeled".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\ninterface ItemProps { tone?: string }\nexport function Root(props: Props) { return null; }\nfunction Item(props: ItemProps) { return null; }\nfunction configure(Item: number) { Root.Item = <number>Item; }".to_owned(),
    ));
    cases.push((
        "sealed: a member assigned through an assertion, declared once | plain".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\ninterface ItemProps { tone?: string }\nexport function Root(props: Props) { return null; }\nfunction Item(props: ItemProps) { return null; }\nRoot.Item = Item;".to_owned(),
    ));
    cases.push((
        "sealed: a member assigned through an assertion, declared once | peeled".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\ninterface ItemProps { tone?: string }\nexport function Root(props: Props) { return null; }\nfunction Item(props: ItemProps) { return null; }\nRoot.Item = <typeof Item>Item;".to_owned(),
    ));
    cases.push((
        "sealed: a wrapper named by convention, a local of its name (finding 3) | plain".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\nexport function Root(props: Props) { return null; }\nconst memo = (_: typeof Root) => Symbol();\nexport const PublicRoot = memo(Root);".to_owned(),
    ));
    cases.push((
        "sealed: a wrapper named by convention, a local of its name (finding 3) | peeled".to_owned(),
        "// ts\nimport React from 'react';\ninterface Props { label?: string }\nexport function Root(props: Props) { return null; }\nconst memo = (_: typeof Root) => Symbol();\nexport const PublicRoot = (<typeof memo>memo)(Root);".to_owned(),
    ));
    cases.push((
        "sealed: a wrapper named by convention, React's import | plain".to_owned(),
        "// ts\nimport { memo } from 'react';\ninterface Props { label?: string }\nexport function Root(props: Props) { return null; }\nexport const PublicRoot = memo(Root);".to_owned(),
    ));
    cases.push((
        "sealed: a wrapper named by convention, React's import | peeled".to_owned(),
        "// ts\nimport { memo } from 'react';\ninterface Props { label?: string }\nexport function Root(props: Props) { return null; }\nexport const PublicRoot = (<typeof memo>memo)(Root);".to_owned(),
    ));
    cases.push((
        "sealed: a destructured import shadowed by a namespace local (finding 4) | plain".to_owned(),
        "// ts\nimport * as Ui from './ui';\nnamespace N { const Ui = { Button: 123 }; export const { Button: PublicButton } = Ui; }".to_owned(),
    ));
    cases.push((
        "sealed: a destructured import shadowed by a namespace local (finding 4) | peeled".to_owned(),
        "// ts\nimport * as Ui from './ui';\nnamespace N { const Ui = { Button: 123 }; export const { Button: PublicButton } = <typeof Ui>Ui; }".to_owned(),
    ));
    cases.push((
        "sealed: a destructured import declared once | plain".to_owned(),
        "// ts\nimport * as Ui from './ui';\nexport const { Button: PublicButton } = Ui;".to_owned(),
    ));
    cases.push((
        "sealed: a destructured import declared once | peeled".to_owned(),
        "// ts\nimport * as Ui from './ui';\nexport const { Button: PublicButton } = <typeof Ui>Ui;".to_owned(),
    ));
    cases.push((
        "writes: a later source replaces assign (finding 1) | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, { assign: (target: object) => target });\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a later source replaces assign (finding 1) | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, { assign: (target: object) => target });\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a later source as const replaces assign | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, ({ assign: (target: object) => target }) as const);\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a later source as const replaces assign | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, ({ assign: (target: object) => target }) as const);\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a later source satisfies replaces assign | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, { assign: (target: object) => target } satisfies object);\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a later source satisfies replaces assign | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, { assign: (target: object) => target } satisfies object);\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a spread in a later source | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, { ...extra });\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a spread in a later source | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, { ...extra });\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: an unreadable later source | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, extra);\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: an unreadable later source | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, extra);\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a spread argument | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, ...sources);\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: a spread argument | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, ...sources);\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: defineProperties with a later descriptor | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.defineProperties(Object, {}, { assign: { value: 1 } });\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: defineProperties with a later descriptor | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.defineProperties(Object, {}, { assign: { value: 1 } });\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: later sources with unrelated keys | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, { other: 1 });\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "writes: later sources with unrelated keys | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\ndeclare const extra: object;\ndeclare const sources: object[];\nObject.assign(Object, {}, { other: 1 });\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "members: a namespace-local member behind <T> (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; export const Root = Object.assign(Impl, { Item: <number>Item }); }".to_owned(),
    ));
    cases.push((
        "members: declarator, bare, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: declarator, bare, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: declarator, as, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item: (Item as typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: declarator, as, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, { Item: (Item as typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: declarator, assertion, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: declarator, assertion, plain call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "members: declarator, assertion, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: declarator, assertion, plain call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "members: declarator, satisfies, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item: (Item satisfies typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: declarator, satisfies, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, { Item: (Item satisfies typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: declarator, non-null, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item: Item! });".to_owned(),
    ));
    cases.push((
        "members: declarator, non-null, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, { Item: Item! });".to_owned(),
    ));
    cases.push((
        "members: declarator, parens, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item: (Item) });".to_owned(),
    ));
    cases.push((
        "members: declarator, parens, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, { Item: (Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, bare, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, bare, peeled call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, bare, peeled call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\n(Object.assign as typeof Object.assign)(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, bare, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, bare, peeled call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, bare, peeled call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\n(Object.assign as typeof Object.assign)(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, as, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, as, plain call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: (Item as typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, as, peeled call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, as, peeled call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\n(Object.assign as typeof Object.assign)(Root, { Item: (Item as typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, as, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, as, plain call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: (Item as typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, as, peeled call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, as, peeled call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\n(Object.assign as typeof Object.assign)(Root, { Item: (Item as typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, assertion, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, assertion, plain call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "members: statement, assertion, peeled call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, assertion, peeled call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\n(Object.assign as typeof Object.assign)(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "members: statement, assertion, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, assertion, plain call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "members: statement, assertion, peeled call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, assertion, peeled call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\n(Object.assign as typeof Object.assign)(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "members: statement, satisfies, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, satisfies, plain call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: (Item satisfies typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, satisfies, peeled call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, satisfies, peeled call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\n(Object.assign as typeof Object.assign)(Root, { Item: (Item satisfies typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, satisfies, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, satisfies, plain call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: (Item satisfies typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, satisfies, peeled call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, satisfies, peeled call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\n(Object.assign as typeof Object.assign)(Root, { Item: (Item satisfies typeof Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, non-null, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, non-null, plain call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item! });".to_owned(),
    ));
    cases.push((
        "members: statement, non-null, peeled call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, non-null, peeled call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\n(Object.assign as typeof Object.assign)(Root, { Item: Item! });".to_owned(),
    ));
    cases.push((
        "members: statement, non-null, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, non-null, plain call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item! });".to_owned(),
    ));
    cases.push((
        "members: statement, non-null, peeled call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, non-null, peeled call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\n(Object.assign as typeof Object.assign)(Root, { Item: Item! });".to_owned(),
    ));
    cases.push((
        "members: statement, parens, plain call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: (Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, parens, peeled call, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, parens, peeled call, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\n(Object.assign as typeof Object.assign)(Root, { Item: (Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, parens, plain call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: (Item) });".to_owned(),
    ));
    cases.push((
        "members: statement, parens, peeled call, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nObject.assign(Root, { Item: Item });".to_owned(),
    ));
    cases.push((
        "members: statement, parens, peeled call, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\n(Object.assign as typeof Object.assign)(Root, { Item: (Item) });".to_owned(),
    ));
    cases.push((
        "targets: a target behind <T>, at module level | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "targets: a target behind <T>, at module level | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nObject.assign(<typeof Root>Root, { Item });".to_owned(),
    ));
    cases.push((
        "targets: a target behind <T>, in a function | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction attach() { Object.assign(Root, { Item }); }".to_owned(),
    ));
    cases.push((
        "targets: a target behind <T>, in a function | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction attach() { Object.assign(<typeof Root>Root, { Item }); }".to_owned(),
    ));
    cases.push((
        "targets: a target behind <T>, bound in a function | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nexport function attach() { const Bound = Object.assign(Root, { Item }); return Bound; }".to_owned(),
    ));
    cases.push((
        "targets: a target behind <T>, bound in a function | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nexport function attach() { const Bound = Object.assign(<typeof Root>Root, { Item }); return Bound; }".to_owned(),
    ));
    cases.push((
        "targets: a target behind <T>, beside a write to Object.assign | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Root(props: Props) { return createElement('div'); }\nObject.assign(Object, {}, { assign: (target: object) => target });\nexport const Public = Object.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "targets: a target behind <T>, beside a write to Object.assign | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Root(props: Props) { return createElement('div'); }\nObject.assign(Object, {}, { assign: (target: object) => target });\nexport const Public = Object.assign(<typeof Root>Root, { Item });".to_owned(),
    ));
    cases.push((
        "objects: an object behind <const>, unique | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "objects: an object behind <const>, unique | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "objects: an object behind <const>, ambiguous | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "objects: an object behind <const>, ambiguous | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nnamespace N { const Item = 123; }\nexport const Root = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "parents: a member assigned to a parent a parameter of its name (finding 1) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction configure(Root: { Item?: typeof Item }) { Root.Item = Item; }".to_owned(),
    ));
    cases.push((
        "parents: a member assigned to a parent a parameter of its name (finding 1) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction configure(Root: { Item?: typeof Item }) { Root.Item = <typeof Item>Item; }".to_owned(),
    ));
    cases.push((
        "parents: a member assigned to a parent declared once, in a function | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction configure() { Root.Item = Item; }".to_owned(),
    ));
    cases.push((
        "parents: a member assigned to a parent declared once, in a function | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction configure() { Root.Item = <typeof Item>Item; }".to_owned(),
    ));
    cases.push((
        "receivers: Promise.resolve, a global | plain".to_owned(),
        "export const Button = (props: Props) => Promise.resolve(<div />);".to_owned(),
    ));
    cases.push((
        "receivers: Promise.resolve, a global | whole".to_owned(),
        "export const Button = (props: Props) => (Promise.resolve as typeof Promise.resolve)(<div />);".to_owned(),
    ));
    cases.push((
        "receivers: Promise.resolve, a global | receiver".to_owned(),
        "export const Button = (props: Props) => (Promise as typeof Promise).resolve(<div />);".to_owned(),
    ));
    cases.push((
        "receivers: Promise.resolve, a global | both".to_owned(),
        "export const Button = (props: Props) => ((Promise as typeof Promise).resolve as typeof Promise.resolve)(<div />);".to_owned(),
    ));
    cases.push((
        "receivers: Promise.resolve, an import of another module (finding 2) | plain".to_owned(),
        "import Promise from './codec';\nexport const Button = (props: Props) => Promise.resolve(<div />);".to_owned(),
    ));
    cases.push((
        "receivers: Promise.resolve, an import of another module (finding 2) | whole".to_owned(),
        "import Promise from './codec';\nexport const Button = (props: Props) => (Promise.resolve as typeof Promise.resolve)(<div />);".to_owned(),
    ));
    cases.push((
        "receivers: Promise.resolve, an import of another module (finding 2) | receiver".to_owned(),
        "import Promise from './codec';\nexport const Button = (props: Props) => (Promise as typeof Promise).resolve(<div />);".to_owned(),
    ));
    cases.push((
        "receivers: Promise.resolve, an import of another module (finding 2) | both".to_owned(),
        "import Promise from './codec';\nexport const Button = (props: Props) => ((Promise as typeof Promise).resolve as typeof Promise.resolve)(<div />);".to_owned(),
    ));
    cases.push((
        "receivers: React.createElement, React's import | plain".to_owned(),
        "import * as React from 'react';\nexport const Button = (props: Props) => React.createElement('div');"
            .to_owned(),
    ));
    cases.push((
        "receivers: React.createElement, React's import | whole".to_owned(),
        "import * as React from 'react';\nexport const Button = (props: Props) => (React.createElement as typeof React.createElement)('div');".to_owned(),
    ));
    cases.push((
        "receivers: React.createElement, React's import | receiver".to_owned(),
        "import * as React from 'react';\nexport const Button = (props: Props) => (React as typeof React).createElement('div');".to_owned(),
    ));
    cases.push((
        "receivers: React.createElement, React's import | both".to_owned(),
        "import * as React from 'react';\nexport const Button = (props: Props) => ((React as typeof React).createElement as typeof React.createElement)('div');".to_owned(),
    ));
    cases.push((
        "receivers: React.createElement, an import of another module | plain".to_owned(),
        "import * as React from './codec';\nexport const Button = (props: Props) => React.createElement('div');"
            .to_owned(),
    ));
    cases.push((
        "receivers: React.createElement, an import of another module | whole".to_owned(),
        "import * as React from './codec';\nexport const Button = (props: Props) => (React.createElement as typeof React.createElement)('div');".to_owned(),
    ));
    cases.push((
        "receivers: React.createElement, an import of another module | receiver".to_owned(),
        "import * as React from './codec';\nexport const Button = (props: Props) => (React as typeof React).createElement('div');".to_owned(),
    ));
    cases.push((
        "receivers: React.createElement, an import of another module | both".to_owned(),
        "import * as React from './codec';\nexport const Button = (props: Props) => ((React as typeof React).createElement as typeof React.createElement)('div');".to_owned(),
    ));
    cases.push((
        "receivers: Preact.h, Preact's import | plain".to_owned(),
        "import * as Preact from 'preact';\nexport const Button = (props: Props) => Preact.h('div');".to_owned(),
    ));
    cases.push((
        "receivers: Preact.h, Preact's import | whole".to_owned(),
        "import * as Preact from 'preact';\nexport const Button = (props: Props) => (Preact.h as typeof Preact.h)('div');".to_owned(),
    ));
    cases.push((
        "receivers: Preact.h, Preact's import | receiver".to_owned(),
        "import * as Preact from 'preact';\nexport const Button = (props: Props) => (Preact as typeof Preact).h('div');".to_owned(),
    ));
    cases.push((
        "receivers: Preact.h, Preact's import | both".to_owned(),
        "import * as Preact from 'preact';\nexport const Button = (props: Props) => ((Preact as typeof Preact).h as typeof Preact.h)('div');".to_owned(),
    ));
    cases.push((
        "receivers: Preact.h, an import of another module | plain".to_owned(),
        "import * as Preact from './codec';\nexport const Button = (props: Props) => Preact.h('div');".to_owned(),
    ));
    cases.push((
        "receivers: Preact.h, an import of another module | whole".to_owned(),
        "import * as Preact from './codec';\nexport const Button = (props: Props) => (Preact.h as typeof Preact.h)('div');".to_owned(),
    ));
    cases.push((
        "receivers: Preact.h, an import of another module | receiver".to_owned(),
        "import * as Preact from './codec';\nexport const Button = (props: Props) => (Preact as typeof Preact).h('div');".to_owned(),
    ));
    cases.push((
        "receivers: Preact.h, an import of another module | both".to_owned(),
        "import * as Preact from './codec';\nexport const Button = (props: Props) => ((Preact as typeof Preact).h as typeof Preact.h)('div');".to_owned(),
    ));
    cases.push((
        "receivers: Object.assign, a global | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "receivers: Object.assign, a global | whole".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "receivers: Object.assign, a global | receiver".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object as typeof Object).assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "receivers: Object.assign, a global | both".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n((Object as typeof Object).assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "receivers: Object.assign, an import of another module | plain".to_owned(),
        "import Object from './codec';\ninterface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "receivers: Object.assign, an import of another module | whole".to_owned(),
        "import Object from './codec';\ninterface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "receivers: Object.assign, an import of another module | receiver".to_owned(),
        "import Object from './codec';\ninterface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n(Object as typeof Object).assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "receivers: Object.assign, an import of another module | both".to_owned(),
        "import Object from './codec';\ninterface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\n((Object as typeof Object).assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "shapes: an object behind <const> before a later source (finding 1) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item }, { Item: null });".to_owned(),
    ));
    cases.push((
        "shapes: an object behind <const> before a later source (finding 1) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, <const>{ Item }, { Item: null });".to_owned(),
    ));
    cases.push((
        "shapes: a member behind <T> before a later source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item }, { Item: null });".to_owned(),
    ));
    cases.push((
        "shapes: a member behind <T> before a later source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item: <typeof Item>Item }, { Item: null });".to_owned(),
    ));
    cases.push((
        "shapes: a target behind <T> before a later source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(Impl, { Item }, { Item: null });".to_owned(),
    ));
    cases.push((
        "shapes: a target behind <T> before a later source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Root = Object.assign(<typeof Impl>Impl, { Item }, { Item: null });".to_owned(),
    ));
    cases.push((
        "keys: a template key with a hex escape (finding 2) | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject[`ass\\x69gn`] = (target: object) => target;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a template key with a hex escape (finding 2) | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject[`ass\\x69gn`] = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a string key with a hex escape | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['ass\\x69gn'] = (target: object) => target;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a string key with a hex escape | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['ass\\x69gn'] = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a template key with a unicode escape | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject[`ass\\u0069gn`] = (target: object) => target;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a template key with a unicode escape | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject[`ass\\u0069gn`] = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a string key with a code point escape | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['ass\\u{69}gn'] = (target: object) => target;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a string key with a code point escape | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['ass\\u{69}gn'] = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a string key with a line continuation | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['ass\\\nign'] = (target: object) => target;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a string key with a line continuation | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['ass\\\nign'] = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a concatenated key | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['ass' + 'ign'] = (target: object) => target;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a concatenated key | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject['ass' + 'ign'] = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a key in a constant | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nconst key = 'assign';\nObject[key] = (target: object) => target;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: a key in a constant | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nconst key = 'assign';\nObject[key] = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: Reflect.set with an escaped key | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nReflect.set(Object, `ass\\x69gn`, (target: object) => target);\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: Reflect.set with an escaped key | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nReflect.set(Object, `ass\\x69gn`, (target: object) => target);\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: an escaped key that is another member | plain".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject[`oth\\x65r`] = 1;\nObject.assign(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "keys: an escaped key that is another member | peeled".to_owned(),
        "interface ItemProps { tone?: string }\nexport function Root(props: Props) { return <div />; }\nObject[`oth\\x65r`] = 1;\n(Object.assign as typeof Object.assign)(Root, { Item: (p: ItemProps) => <div /> });".to_owned(),
    ));
    cases.push((
        "callees: forwardRef, a bare callee | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { forwardRef } from 'react';\nexport const Button = forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: forwardRef, a bare callee | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { forwardRef } from 'react';\nexport const Button = (<typeof forwardRef>forwardRef)<HTMLButtonElement, Props>((props, ref) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: forwardRef, a member callee on its module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const Button = React.forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: forwardRef, a member callee on its module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const Button = (<typeof React.forwardRef>React.forwardRef)<HTMLButtonElement, Props>((props, ref) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: forwardRef, a member callee on another module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = codec.forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: forwardRef, a member callee on another module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = (<typeof codec.forwardRef>codec.forwardRef)<HTMLButtonElement, Props>((props, ref) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: forwardRef, a member callee on an aliased namespace | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const Button = R.forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: forwardRef, a member callee on an aliased namespace | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const Button = (<typeof R.forwardRef>R.forwardRef)<HTMLButtonElement, Props>((props, ref) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: memo, a bare callee | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { memo } from 'react';\nexport const Button = memo((props: Props) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: memo, a bare callee | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { memo } from 'react';\nexport const Button = (<typeof memo>memo)((props: Props) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: memo, a member callee on its module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const Button = React.memo((props: Props) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: memo, a member callee on its module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const Button = (<typeof React.memo>React.memo)((props: Props) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: memo, a member callee on another module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = codec.memo((props: Props) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: memo, a member callee on another module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = (<typeof codec.memo>codec.memo)((props: Props) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: memo, a member callee on an aliased namespace | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const Button = R.memo((props: Props) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: memo, a member callee on an aliased namespace | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const Button = (<typeof R.memo>R.memo)((props: Props) => createElement('div'));".to_owned(),
    ));
    cases.push((
        "callees: lazy, a bare callee | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { lazy } from 'react';\nexport const Button = lazy(() => import('./Impl'));".to_owned(),
    ));
    cases.push((
        "callees: lazy, a bare callee | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { lazy } from 'react';\nexport const Button = (<typeof lazy>lazy)(() => import('./Impl'));".to_owned(),
    ));
    cases.push((
        "callees: lazy, a member callee on its module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const Button = React.lazy(() => import('./Impl'));".to_owned(),
    ));
    cases.push((
        "callees: lazy, a member callee on its module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const Button = (<typeof React.lazy>React.lazy)(() => import('./Impl'));".to_owned(),
    ));
    cases.push((
        "callees: lazy, a member callee on another module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = codec.lazy(() => import('./Impl'));".to_owned(),
    ));
    cases.push((
        "callees: lazy, a member callee on another module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = (<typeof codec.lazy>codec.lazy)(() => import('./Impl'));".to_owned(),
    ));
    cases.push((
        "callees: lazy, a member callee on an aliased namespace | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const Button = R.lazy(() => import('./Impl'));".to_owned(),
    ));
    cases.push((
        "callees: lazy, a member callee on an aliased namespace | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const Button = (<typeof R.lazy>R.lazy)(() => import('./Impl'));".to_owned(),
    ));
    cases.push((
        "callees: createContext, a bare callee | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { createContext } from 'react';\nexport const ButtonContext = createContext<Props | null>(null);".to_owned(),
    ));
    cases.push((
        "callees: createContext, a bare callee | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { createContext } from 'react';\nexport const ButtonContext = (<typeof createContext>createContext)<Props | null>(null);".to_owned(),
    ));
    cases.push((
        "callees: createContext, a member callee on its module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const ButtonContext = React.createContext<Props | null>(null);".to_owned(),
    ));
    cases.push((
        "callees: createContext, a member callee on its module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const ButtonContext = (<typeof React.createContext>React.createContext)<Props | null>(null);".to_owned(),
    ));
    cases.push((
        "callees: createContext, a member callee on another module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const ButtonContext = codec.createContext<Props | null>(null);".to_owned(),
    ));
    cases.push((
        "callees: createContext, a member callee on another module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const ButtonContext = (<typeof codec.createContext>codec.createContext)<Props | null>(null);".to_owned(),
    ));
    cases.push((
        "callees: createContext, a member callee on an aliased namespace | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const ButtonContext = R.createContext<Props | null>(null);".to_owned(),
    ));
    cases.push((
        "callees: createContext, a member callee on an aliased namespace | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const ButtonContext = (<typeof R.createContext>R.createContext)<Props | null>(null);".to_owned(),
    ));
    cases.push((
        "callees: createElement, a bare callee | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nexport const Button = (props: Props) => createElement('div');"
            .to_owned(),
    ));
    cases.push((
        "callees: createElement, a bare callee | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nexport const Button = (props: Props) => (<typeof createElement>createElement)('div');".to_owned(),
    ));
    cases.push((
        "callees: createElement, a member callee on its module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const Button = (props: Props) => React.createElement('div');".to_owned(),
    ));
    cases.push((
        "callees: createElement, a member callee on its module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as React from 'react';\nexport const Button = (props: Props) => (<typeof React.createElement>React.createElement)('div');".to_owned(),
    ));
    cases.push((
        "callees: createElement, a member callee on another module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = (props: Props) => codec.createElement('div');".to_owned(),
    ));
    cases.push((
        "callees: createElement, a member callee on another module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = (props: Props) => (<typeof codec.createElement>codec.createElement)('div');".to_owned(),
    ));
    cases.push((
        "callees: createElement, a member callee on an aliased namespace | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const Button = (props: Props) => R.createElement('div');".to_owned(),
    ));
    cases.push((
        "callees: createElement, a member callee on an aliased namespace | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'react';\nexport const Button = (props: Props) => (<typeof R.createElement>R.createElement)('div');".to_owned(),
    ));
    cases.push((
        "callees: h, a bare callee | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { h } from 'preact';\nexport const Button = (props: Props) => h('div');".to_owned(),
    ));
    cases.push((
        "callees: h, a bare callee | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport { h } from 'preact';\nexport const Button = (props: Props) => (<typeof h>h)('div');".to_owned(),
    ));
    cases.push((
        "callees: h, a member callee on its module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as Preact from 'preact';\nexport const Button = (props: Props) => Preact.h('div');".to_owned(),
    ));
    cases.push((
        "callees: h, a member callee on its module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as Preact from 'preact';\nexport const Button = (props: Props) => (<typeof Preact.h>Preact.h)('div');".to_owned(),
    ));
    cases.push((
        "callees: h, a member callee on another module | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = (props: Props) => codec.h('div');".to_owned(),
    ));
    cases.push((
        "callees: h, a member callee on another module | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as codec from './codec';\nexport const Button = (props: Props) => (<typeof codec.h>codec.h)('div');".to_owned(),
    ));
    cases.push((
        "callees: h, a member callee on an aliased namespace | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'preact';\nexport const Button = (props: Props) => R.h('div');".to_owned(),
    ));
    cases.push((
        "callees: h, a member callee on an aliased namespace | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\nimport * as R from 'preact';\nexport const Button = (props: Props) => (<typeof R.h>R.h)('div');".to_owned(),
    ));
    cases.push((
        "callees: a foreign forwardRef beside React's import (finding 3) | plain".to_owned(),
        "// ts\nimport { createElement, forwardRef } from 'react';\nimport * as codec from './codec';\nexport const Button = codec.forwardRef<HTMLButtonElement, Props>('button');".to_owned(),
    ));
    cases.push((
        "callees: a foreign forwardRef beside React's import (finding 3) | peeled".to_owned(),
        "// ts\nimport { createElement, forwardRef } from 'react';\nimport * as codec from './codec';\nexport const Button = (<typeof codec.forwardRef>codec.forwardRef)<HTMLButtonElement, Props>('button');".to_owned(),
    ));
    cases.push((
        "builtin: no write, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: no write, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: no write, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: no write, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: no write, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: no write, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: no write, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: no write, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a direct write, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a direct write, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a direct write, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a direct write, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign = (target: object) => target;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a direct write, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a direct write, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign = (target: object) => target;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a direct write, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a direct write, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a computed write, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject['assign'] = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a computed write, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject['assign'] = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a computed write, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject['assign'] = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a computed write, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject['assign'] = (target: object) => target;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a computed write, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject['assign'] = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a computed write, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject['assign'] = (target: object) => target;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a computed write, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject['assign'] = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a computed write, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject['assign'] = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through an as receiver (finding 1), peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object as typeof Object).assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through an as receiver (finding 1), peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object as typeof Object).assign = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through an as receiver (finding 1), peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object as typeof Object).assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through an as receiver (finding 1), peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object as typeof Object).assign = (target: object) => target;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through an as receiver (finding 1), peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object as typeof Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through an as receiver (finding 1), peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object as typeof Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through an as receiver (finding 1), peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object as typeof Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through an as receiver (finding 1), peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object as typeof Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a <T> receiver, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(<typeof Object>Object).assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a <T> receiver, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(<typeof Object>Object).assign = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a <T> receiver, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(<typeof Object>Object).assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a <T> receiver, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(<typeof Object>Object).assign = (target: object) => target;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a <T> receiver, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(<typeof Object>Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a <T> receiver, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(<typeof Object>Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a <T> receiver, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(<typeof Object>Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a <T> receiver, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(<typeof Object>Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a non-null receiver, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject!.assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a non-null receiver, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject!.assign = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a non-null receiver, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject!.assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a non-null receiver, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject!.assign = (target: object) => target;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a non-null receiver, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject!.assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a non-null receiver, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject!.assign = (target: object) => target;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a non-null receiver, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject!.assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a non-null receiver, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject!.assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a parenthesized receiver, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object).assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a parenthesized receiver, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object).assign = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a parenthesized receiver, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object).assign = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a parenthesized receiver, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object).assign = (target: object) => target;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a parenthesized receiver, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a parenthesized receiver, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a parenthesized receiver, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write through a parenthesized receiver, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object).assign = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write to a cast target, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as any) = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write to a cast target, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as any) = (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write to a cast target, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as any) = (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write to a cast target, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as any) = (target: object) => target;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write to a cast target, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as any) = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write to a cast target, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as any) = (target: object) => target;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write to a cast target, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as any) = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a write to a cast target, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n(Object.assign as any) = (target: object) => target;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a compound write, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign ||= (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a compound write, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign ||= (target: object) => target;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a compound write, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign ||= (target: object) => target;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a compound write, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign ||= (target: object) => target;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a compound write, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign ||= (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a compound write, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign ||= (target: object) => target;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a compound write, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign ||= (target: object) => target;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a compound write, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\nObject.assign ||= (target: object) => target;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a destructuring write, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n({ assign: Object.assign } = { assign: (target: object) => target });\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a destructuring write, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n({ assign: Object.assign } = { assign: (target: object) => target });\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a destructuring write, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n({ assign: Object.assign } = { assign: (target: object) => target });\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a destructuring write, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n({ assign: Object.assign } = { assign: (target: object) => target });\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a destructuring write, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n({ assign: Object.assign } = { assign: (target: object) => target });\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a destructuring write, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n({ assign: Object.assign } = { assign: (target: object) => target });\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a destructuring write, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n({ assign: Object.assign } = { assign: (target: object) => target });\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a destructuring write, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n({ assign: Object.assign } = { assign: (target: object) => target });\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: an array destructuring write, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n[Object.assign] = [(target: object) => target];\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: an array destructuring write, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n[Object.assign] = [(target: object) => target];\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: an array destructuring write, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n[Object.assign] = [(target: object) => target];\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: an array destructuring write, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n[Object.assign] = [(target: object) => target];\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: an array destructuring write, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n[Object.assign] = [(target: object) => target];\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: an array destructuring write, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n[Object.assign] = [(target: object) => target];\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: an array destructuring write, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n[Object.assign] = [(target: object) => target];\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: an array destructuring write, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\n[Object.assign] = [(target: object) => target];\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a delete, peeled call | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\ndelete (Object as any).assign;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a delete, peeled call | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\ndelete (Object as any).assign;\n(Object.assign as typeof Object.assign)(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a delete, peeled member | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\ndelete (Object as any).assign;\nObject.assign(Root, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a delete, peeled member | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\ndelete (Object as any).assign;\nObject.assign(Root, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "builtin: a delete, peeled source | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\ndelete (Object as any).assign;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a delete, peeled source | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\ndelete (Object as any).assign;\nexport const Public = Object.assign(Impl, <const>{ Item });".to_owned(),
    ));
    cases.push((
        "builtin: a delete, peeled member, bound (finding 2) | plain".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\ndelete (Object as any).assign;\nexport const Public = Object.assign(Impl, { Item });".to_owned(),
    ));
    cases.push((
        "builtin: a delete, peeled member, bound (finding 2) | peeled".to_owned(),
        "// ts\nimport { createElement } from 'react';\ninterface ItemProps { tone?: string }\nfunction Item(props: ItemProps) { return createElement('span'); }\nexport function Root(props: Props) { return createElement('div'); }\nfunction Impl(props: Props) { return createElement('div'); }\ndelete (Object as any).assign;\nexport const Public = Object.assign(Impl, { Item: <typeof Item>Item });".to_owned(),
    ));
    cases.push((
        "pascal: a function returning a symbol (finding 3) | plain".to_owned(),
        "import * as React from 'react';\nfunction MakeToken(): symbol { return Symbol(); }\nexport const Button = (props: Props) => MakeToken();".to_owned(),
    ));
    cases.push((
        "pascal: a function returning a symbol (finding 3) | peeled".to_owned(),
        "import * as React from 'react';\nfunction MakeToken(): symbol { return Symbol(); }\nexport const Button = (props: Props) => (MakeToken as typeof MakeToken)();".to_owned(),
    ));
    cases.push((
        "pascal: a helper declared to return an element | plain".to_owned(),
        "import * as React from 'react';\nfunction MakeCard(): React.ReactElement { return <div />; }\nexport const Button = (props: Props) => MakeCard();".to_owned(),
    ));
    cases.push((
        "pascal: a helper declared to return an element | peeled".to_owned(),
        "import * as React from 'react';\nfunction MakeCard(): React.ReactElement { return <div />; }\nexport const Button = (props: Props) => (MakeCard as typeof MakeCard)();".to_owned(),
    ));
    cases.push((
        "pascal: a helper of a name declared twice | plain".to_owned(),
        "import * as React from 'react';\nfunction MakeCard(): React.ReactElement { return <div />; }\nnamespace N { export const MakeCard = 1; }\nexport const Button = (props: Props) => MakeCard();".to_owned(),
    ));
    cases.push((
        "pascal: a helper of a name declared twice | peeled".to_owned(),
        "import * as React from 'react';\nfunction MakeCard(): React.ReactElement { return <div />; }\nnamespace N { export const MakeCard = 1; }\nexport const Button = (props: Props) => (MakeCard as typeof MakeCard)();".to_owned(),
    ));
    cases.push((
        "pascal: a function without a return type | plain".to_owned(),
        "import * as React from 'react';\nfunction MakeCard() { return <div />; }\nexport const Button = (props: Props) => MakeCard();".to_owned(),
    ));
    cases.push((
        "pascal: a function without a return type | peeled".to_owned(),
        "import * as React from 'react';\nfunction MakeCard() { return <div />; }\nexport const Button = (props: Props) => (MakeCard as typeof MakeCard)();".to_owned(),
    ));
    cases
}

/// Every mapping of a row and every diagnostic, sorted, as `parity_output` prints them.
fn factory_parity_output(source: &str) -> String {
    let path = if source.starts_with("// ts\n") { "/fixtures/Button.ts" } else { "/fixtures/Button.tsx" };
    let data = parse_file(Utf8Path::new(path), source);
    let mut lines: Vec<String> = data
        .component_mappings
        .iter()
        .map(|m| {
            // The export names too, so a default export that falls back to the component its call wraps is compared.
            let exports: Vec<String> = m
                .exports
                .iter()
                .map(|export| {
                    std::iter::once(export.name.as_str())
                        .chain(export.members.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(".")
                })
                .collect();
            let rule = serde_json::to_string(&m.rule).unwrap_or_default();
            let exported =
                if exports.is_empty() { String::new() } else { format!(" exported as {}", exports.join(",")) };
            // The callee whose declared result the resolver reads, if any.
            let via = m.wrapper_callee.as_ref().map(|callee| format!(" via {}", callee.name)).unwrap_or_default();
            // The defaults a static `defaultProps` assignment supplies.
            let mut defaults: Vec<&str> = m.param_defaults.keys().map(String::as_str).collect();
            defaults.sort_unstable();
            let defaults =
                if defaults.is_empty() { String::new() } else { format!(" defaults {}", defaults.join(",")) };
            format!("{}:{}{rule}{exported}{via}{defaults}", m.component_name, m.props_type_name)
        })
        .chain(data.diagnostics.iter().map(|d| format!("{:?} {:?}: {}", d.severity, d.code, d.message)))
        .collect();
    lines.sort();
    lines.join(" | ")
}

// END FACTORY PARITY CASES.

/// Round 17, parity: a known factory is recognised exactly as origin/main recognised it, by main's own predicates on
/// the callee, and a cast never makes a call a factory main did not know. The oracle is `factory_parity_base.tsv`,
/// printed from the block above on origin/main f4a88f93 (oxc 0.158). Every row written without a cast extracts exactly
/// what main extracts, right or wrong; the rows that pin a known wrong answer of main's are the follow-up's. A row
/// written through a cast either equals main's row or, where main read no callee through the cast, main's plain call
/// (the one linear peel reads a cast that keeps the callee's type as the plain call); those rows are counted.
#[test]
fn a_known_factory_is_recognised_exactly_as_origin_main_recognised_it() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases = factory_parity_cases();
    assert_eq!(base.len(), cases.len(), "the base table and the cases are out of step");
    let mut failures = Vec::new();
    let mut read_as_the_plain_call = 0;
    for (id, source) in &cases {
        let now = factory_parity_output(source);
        let main = base.get(id.as_str()).copied().unwrap_or("not in the base table");
        // Through the namespace cast main attached the callee name `unknown.withContext`, which names no declaration;
        // this head attaches none (round 6: a callee asserted to another type has no declaration to read).
        let main_without_garbage_callee;
        let main = if id.as_str() == "replacing cast: a member generic wrapper" {
            main_without_garbage_callee = main.replace(" via unknown.withContext", "");
            main_without_garbage_callee.as_str()
        } else {
            main
        };
        let cast = id.rsplit_once(" | ").filter(|(_, cast)| *cast != "plain");
        // A component main listed that this head does not: never allowed, whatever main's plain call gives.
        let mappings = |output: &str| -> Vec<String> {
            output
                .split(" | ")
                .filter(|line| !line.starts_with("Info "))
                .filter_map(|line| line.split_once(':').map(|(name, _)| name.to_owned()))
                .collect()
        };
        let loses = mappings(main).iter().any(|mapping| !mappings(&now).contains(mapping));
        let holds = match cast {
            None => now == main,
            Some((row, _)) => {
                let main_plain = base.get(format!("{row} | plain").as_str()).copied().unwrap_or(main);
                // The same components as main, or main's plain call where main read no callee through the cast. A
                // tagged claim that falls through keeps its skip beside the factory's component (round 18).
                let components = |output: &str| -> Vec<String> {
                    output.split(" | ").filter(|line| !line.starts_with("Info ")).map(str::to_owned).collect()
                };
                components(&now) == components(main) || (main != main_plain && now == main_plain && !loses)
            }
        };
        if !holds {
            failures.push(format!("{id}:\n  main {main}\n  now  {now}"));
        } else if now != main {
            read_as_the_plain_call += 1;
        }
    }
    assert!(failures.is_empty(), "{} rows:\n{}", failures.len(), failures.join("\n"));
    // Round 26 adds one: `(wrap as typeof wrap)<Props>(render)` with `wrap` declared once keeps the callee, as main's
    // plain call does (`a_callee_through_a_cast_is_attached_only_when_its_name_is_declared_once`). Round 27 adds three:
    // an `Object.assign` through a cast at module level
    // (`an_object_assign_through_a_cast_attaches_only_at_module_level_to_a_parent_declared_once`). Round 28 adds one: a
    // member callee's receiver read through a cast when declared once
    // (`a_receiver_through_a_cast_is_read_only_for_a_callee_and_only_when_declared_once`). Round 29 adds three: an
    // alias, a callee and a heritage whose receiver is declared once
    // (`every_receiver_read_through_a_cast_is_read_only_when_declared_once`). Round 30 adds four: a peeled base or
    // `Object.assign` whose names are trusted (`a_peeled_candidate_is_accepted_only_when_its_names_are_trusted`).
    // Round 31 adds three: a peeled receiver that resolves to its well-known global or import
    // (`a_well_known_name_on_a_peeled_path_must_resolve_to_it`). Round 32 adds two: a peeled `Object.assign` whose
    // target is trusted, bound and exported by default (`an_object_assign_call_is_read_by_its_argument_roles`).
    // Round 33 adds three: a forwardRef or createElement import behind `<T>x` that resolves
    // (`every_well_known_entry_resolves_before_a_peeled_match`).
    // Round 34 adds two: a peeled `Object.assign` beside a write to another `Object` member, and an identifier alias behind
    // `<T>x` declared once (`a_peeled_candidate_relies_only_on_names_the_binding_pass_vouches_for`).
    // Round 36 removes 28: an element factory written through a cast and bound to anything but React's or Preact's import
    // (a destructuring, a `require`, a local function, a relative module, emotion's `jsx`) is read as origin/main read
    // it, so it is no factory (`a_sealed_peel_reads_a_name_only_as_main_read_it_or_once_it_resolves`).
    // Round 36 adds three back: a member assigned through `<T>x` declared once, a `memo` imported from React and a
    // factory on React's receiver behind a cast.
    // Round 37 adds two: a `map` on an array literal and an element helper declared once, both behind a cast
    // (`an_evidence_predicate_through_a_cast_needs_binding_evidence`).
    // Round 38 adds eight: a cast `Object.assign` beside later `Object` sources with unrelated keys
    // (`every_source_of_an_object_mutation_counts_as_a_write`), five members exposed only by a peel and declared once,
    // a target behind `<T>x` at module level and an object literal behind `<const>` whose member is declared once
    // (`an_object_assign_member_is_recognised_by_its_own_read`).
    // Round 38 adds twelve more: cast forms of a well-known member callee whose receiver resolves to its entry, and a
    // member assigned through `<T>x` to a parent declared once (`a_peeled_name_relies_on_a_parent_and_receiver_that_resolve`).
    // Round 39 adds eleven: a well-known member callee behind `<T>x` whose receiver resolves, and an escaped key that is
    // another member (`a_peeled_child_relies_on_the_whole_call_its_key_and_its_receiver`).
    // Round 40 adds six: a cast `Object.assign` part with no write to it in four contexts, and a PascalCase callee behind a
    // cast whose declaration renders by its return type or body (`a_peeled_part_relies_on_the_built_in_and_on_render_evidence`).
    assert_eq!(read_as_the_plain_call, 80);
}

/// Round 28: a member callee's receiver written through a cast is read through it (`ui.wrap`) only when the file
/// declares the receiver's root once; otherwise it is named as origin/main named it (`unknown.wrap`). A static
/// assignment's target (`defaultProps`, `displayName`, a member set to a component or to an inline one) is never read
/// through a cast, as on origin/main, whether a parameter shadows the component or not.
#[test]
fn a_receiver_through_a_cast_is_read_only_for_a_callee_and_only_when_declared_once() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    let now = |id: &str| factory_parity_output(&cases[id]);
    let main = |id: &str| base[id];
    let receiver = "member callee receiver unshadowed";
    assert_eq!(now(&format!("{receiver} | plain")), main(&format!("{receiver} | plain")));
    assert!(main(&format!("{receiver} | typeof itself")).ends_with("via unknown.wrap"));
    assert_eq!(now(&format!("{receiver} | typeof itself")), main(&format!("{receiver} | plain")), "declared once");
    let shadowed = "member callee receiver shadowed by a namespace local";
    for cast in ["plain", "typeof itself"] {
        let id = format!("{shadowed} | {cast}");
        assert_eq!(now(&id), main(&id), "{id}");
    }
    for kind in ["defaultProps", "displayName", "a member set to a component", "a member set to an inline component"] {
        for shadow in ["unshadowed", "shadowed by a parameter"] {
            for cast in ["plain", "typeof itself"] {
                let id = format!("static {kind} {shadow} | {cast}");
                assert_eq!(now(&id), main(&id), "{id}");
            }
        }
    }
    // The finding: through the cast, a parameter's `defaultProps` does not reach the exported `Button`.
    assert!(!main("static defaultProps shadowed by a parameter | typeof itself").contains("defaults"));
}

/// Round 30: a candidate only a peel makes readable (`Recognition::Peeled`) is accepted only when every bare name it relies
/// on can mean one declaration, and a base expected to be React's is no local declaration. Each pair's plain form is
/// origin/main's; the peeled form reads as the plain form where its names are trusted, and as origin/main read it
/// (not at all) otherwise.
#[test]
fn a_peeled_candidate_is_accepted_only_when_its_names_are_trusted() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, trusted) in [
        ("heritage, a local class (finding 1)", false),
        ("heritage, an import declared once", true),
        ("heritage, a global", true),
        ("Object.assign member, declared twice (finding 2)", false),
        ("Object.assign member, declared once", true),
        ("Object.assign object behind as const, a shadowing local (finding 3)", false),
        ("Object.assign object behind as const, at module level", true),
        ("Object.assign object behind satisfies, a shadowing local", false),
        ("factory callee behind an instantiation, declared once", true),
        ("factory callee behind an instantiation, a parameter of its name", false),
    ] {
        let plain = format!("recognition: {row} | plain");
        let peeled = format!("recognition: {row} | peeled");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if trusted { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
}

/// Round 40: any part of an `Object.assign` call only a peel exposes (the callee, a member, a source) counts only while
/// `Object.assign` is the built-in, and a write to it counts in every form: through a wrapped receiver or target, as a
/// compound or destructuring assignment, or a `delete`. A PascalCase call is an element by its spelling only as origin/main
/// read the callee; a callee only a peel exposes needs an element helper its name resolves to. Each plain form is
/// origin/main's; a peeled form reads as the plain form only where the evidence holds, and as origin/main read it
/// otherwise.
#[test]
fn a_peeled_part_relies_on_the_built_in_and_on_render_evidence() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    let mut failures = Vec::new();
    for (row, holds) in [
        ("builtin: no write, peeled call", true),
        ("builtin: no write, peeled member", true),
        ("builtin: no write, peeled source", true),
        ("builtin: no write, peeled member, bound (finding 2)", true),
        ("builtin: a direct write, peeled call", false),
        ("builtin: a direct write, peeled member", false),
        ("builtin: a direct write, peeled source", false),
        ("builtin: a direct write, peeled member, bound (finding 2)", false),
        ("builtin: a computed write, peeled call", false),
        ("builtin: a computed write, peeled member", false),
        ("builtin: a computed write, peeled source", false),
        ("builtin: a computed write, peeled member, bound (finding 2)", false),
        ("builtin: a write through an as receiver (finding 1), peeled call", false),
        ("builtin: a write through an as receiver (finding 1), peeled member", false),
        ("builtin: a write through an as receiver (finding 1), peeled source", false),
        ("builtin: a write through an as receiver (finding 1), peeled member, bound (finding 2)", false),
        ("builtin: a write through a <T> receiver, peeled call", false),
        ("builtin: a write through a <T> receiver, peeled member", false),
        ("builtin: a write through a <T> receiver, peeled source", false),
        ("builtin: a write through a <T> receiver, peeled member, bound (finding 2)", false),
        ("builtin: a write through a non-null receiver, peeled call", false),
        ("builtin: a write through a non-null receiver, peeled member", false),
        ("builtin: a write through a non-null receiver, peeled source", false),
        ("builtin: a write through a non-null receiver, peeled member, bound (finding 2)", false),
        ("builtin: a write through a parenthesized receiver, peeled call", false),
        ("builtin: a write through a parenthesized receiver, peeled member", false),
        ("builtin: a write through a parenthesized receiver, peeled source", false),
        ("builtin: a write through a parenthesized receiver, peeled member, bound (finding 2)", false),
        ("builtin: a write to a cast target, peeled call", false),
        ("builtin: a write to a cast target, peeled member", false),
        ("builtin: a write to a cast target, peeled source", false),
        ("builtin: a write to a cast target, peeled member, bound (finding 2)", false),
        ("builtin: a compound write, peeled call", false),
        ("builtin: a compound write, peeled member", false),
        ("builtin: a compound write, peeled source", false),
        ("builtin: a compound write, peeled member, bound (finding 2)", false),
        ("builtin: a destructuring write, peeled call", false),
        ("builtin: a destructuring write, peeled member", false),
        ("builtin: a destructuring write, peeled source", false),
        ("builtin: a destructuring write, peeled member, bound (finding 2)", false),
        ("builtin: an array destructuring write, peeled call", false),
        ("builtin: an array destructuring write, peeled member", false),
        ("builtin: an array destructuring write, peeled source", false),
        ("builtin: an array destructuring write, peeled member, bound (finding 2)", false),
        ("builtin: a delete, peeled call", false),
        ("builtin: a delete, peeled member", false),
        ("builtin: a delete, peeled source", false),
        ("builtin: a delete, peeled member, bound (finding 2)", false),
        ("pascal: a function returning a symbol (finding 3)", false),
        ("pascal: a helper declared to return an element", true),
        ("pascal: a helper of a name declared twice", false),
        // Its body renders: the evidence a callee only a peel exposes needs.
        ("pascal: a function without a return type", true),
    ] {
        let (plain, peeled) = (format!("{row} | plain"), format!("{row} | peeled"));
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if holds { base[plain.as_str()] } else { base[peeled.as_str()] };
        let now = factory_parity_output(&cases[&peeled]);
        if now != expected {
            failures.push(format!("{peeled}:\n  expected {expected}\n  now      {now}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Round 39: a child only a peel exposes (an object, a member, a target) is recorded only in an `Object.assign` of the
/// minimal shape, as a later source may overwrite it. A write to `Object.assign` is read by its key's cooked value, and a
/// key that is not statically known may be any key. A member callee through a wrapper main did not read is React's only
/// as its receiver resolves, and never by a bare import of the same name. Each plain form is origin/main's; a peeled form
/// reads as the plain form only where the evidence holds, and as origin/main read it otherwise.
#[test]
fn a_peeled_child_relies_on_the_whole_call_its_key_and_its_receiver() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    let mut failures = Vec::new();
    for (row, holds) in [
        ("shapes: an object behind <const> before a later source (finding 1)", false),
        ("shapes: a member behind <T> before a later source", false),
        ("shapes: a target behind <T> before a later source", false),
        ("keys: a template key with a hex escape (finding 2)", false),
        ("keys: a string key with a hex escape", false),
        ("keys: a template key with a unicode escape", false),
        ("keys: a string key with a code point escape", false),
        ("keys: a string key with a line continuation", false),
        ("keys: a concatenated key", false),
        ("keys: a key in a constant", false),
        ("keys: Reflect.set with an escaped key", false),
        ("keys: an escaped key that is another member", true),
        ("callees: forwardRef, a bare callee", true),
        ("callees: forwardRef, a member callee on its module", true),
        ("callees: forwardRef, a member callee on another module", false),
        ("callees: forwardRef, a member callee on an aliased namespace", true),
        ("callees: memo, a bare callee", true),
        ("callees: memo, a member callee on its module", true),
        ("callees: memo, a member callee on another module", false),
        ("callees: memo, a member callee on an aliased namespace", true),
        ("callees: lazy, a bare callee", true),
        ("callees: lazy, a member callee on its module", true),
        ("callees: lazy, a member callee on another module", false),
        ("callees: lazy, a member callee on an aliased namespace", true),
        // `createContext` makes no component in any form; through `<T>x` its skip reads as main's peeled row.
        ("callees: createContext, a bare callee", false),
        ("callees: createContext, a member callee on its module", false),
        ("callees: createContext, a member callee on another module", false),
        ("callees: createContext, a member callee on an aliased namespace", false),
        ("callees: createElement, a bare callee", true),
        ("callees: createElement, a member callee on its module", true),
        ("callees: createElement, a member callee on another module", false),
        ("callees: createElement, a member callee on an aliased namespace", true),
        ("callees: h, a bare callee", true),
        ("callees: h, a member callee on its module", true),
        ("callees: h, a member callee on another module", false),
        ("callees: h, a member callee on an aliased namespace", true),
        ("callees: a foreign forwardRef beside React's import (finding 3)", false),
    ] {
        let (plain, peeled) = (format!("{row} | plain"), format!("{row} | peeled"));
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if holds { base[plain.as_str()] } else { base[peeled.as_str()] };
        let now = factory_parity_output(&cases[&peeled]);
        if now != expected {
            failures.push(format!("{peeled}:\n  expected {expected}\n  now      {now}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Round 38: a member assigned through `<T>x` attaches only to a parent that resolves too, as export propagation finds
/// the parent by its bare name (a parameter named `Root` is a second declaration). And a well-known member callee
/// (`Promise.resolve`, `React.createElement`, Preact's `h`, `Object.assign`) is read only when its receiver resolves to
/// the entry in every combination of a peeled whole callee and a peeled receiver: written plainly, as origin/main read
/// it; through any cast, as the plain call only when the receiver is the global or the import it names.
#[test]
fn a_peeled_name_relies_on_a_parent_and_receiver_that_resolve() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, holds) in [
        ("parents: a member assigned to a parent a parameter of its name (finding 1)", false),
        ("parents: a member assigned to a parent declared once, in a function", true),
    ] {
        let (plain, peeled) = (format!("{row} | plain"), format!("{row} | peeled"));
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if holds { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
    for (row, holds) in [
        ("receivers: Promise.resolve, a global", true),
        ("receivers: Promise.resolve, an import of another module (finding 2)", false),
        ("receivers: React.createElement, React's import", true),
        ("receivers: React.createElement, an import of another module", false),
        ("receivers: Preact.h, Preact's import", true),
        ("receivers: Preact.h, an import of another module", false),
        ("receivers: Object.assign, a global", true),
        ("receivers: Object.assign, an import of another module", false),
    ] {
        let plain = format!("{row} | plain");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        for cast in ["whole", "receiver", "both"] {
            let id = format!("{row} | {cast}");
            let expected = if holds { base[plain.as_str()] } else { base[id.as_str()] };
            assert_eq!(factory_parity_output(&cases[&id]), expected, "{id}");
        }
    }
}

/// Round 38: a write to `Object.assign` through any source of an `Object.assign` or `Object.defineProperties` call on
/// `Object` (a later object literal, through `as const` or `satisfies`, a spread, an unreadable source, a spread
/// argument) means a cast `Object.assign` is not the built-in. Each plain form is origin/main's; a peeled form reads as
/// the plain form only where no source may write `assign`, and as origin/main read it otherwise.
#[test]
fn every_source_of_an_object_mutation_counts_as_a_write() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, holds) in [
        ("writes: a later source replaces assign (finding 1)", false),
        ("writes: a later source as const replaces assign", false),
        ("writes: a later source satisfies replaces assign", false),
        ("writes: a spread in a later source", false),
        ("writes: an unreadable later source", false),
        ("writes: a spread argument", false),
        ("writes: defineProperties with a later descriptor", false),
        ("writes: later sources with unrelated keys", true),
    ] {
        let plain = format!("{row} | plain");
        let peeled = format!("{row} | peeled");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if holds { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
}

/// Round 38: an `Object.assign` member value is classified by its own read, not its call's. Read as origin/main read it
/// (bare, `as`, `satisfies`, `!`, parentheses) in a plain call, it is origin/main's whatever else the file declares.
/// Exposed only by a peel (`<T>x`, or any member of a cast call), it counts only when the file declares it once: then
/// the row reads as the plain call with a bare member, and otherwise as origin/main read it. In the statement scan main
/// read a member only as written, so a member behind `as`, `satisfies`, `!` or parentheses stays unread there. A plain
/// call never upgrades a peeled member: finding 2's namespace-local `Item` behind `<number>` stays as main gives it.
#[test]
fn an_object_assign_member_is_recognised_by_its_own_read() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, attaches) in [
        ("members: declarator, bare, plain call, unique | plain", None),
        ("members: declarator, bare, plain call, ambiguous | plain", None),
        ("members: declarator, as, plain call, unique | plain", None),
        ("members: declarator, as, plain call, ambiguous | plain", None),
        ("members: declarator, assertion, plain call, unique", Some(true)),
        ("members: declarator, assertion, plain call, ambiguous", Some(false)),
        ("members: declarator, satisfies, plain call, unique | plain", None),
        ("members: declarator, satisfies, plain call, ambiguous | plain", None),
        ("members: declarator, non-null, plain call, unique | plain", None),
        ("members: declarator, non-null, plain call, ambiguous | plain", None),
        ("members: declarator, parens, plain call, unique | plain", None),
        ("members: declarator, parens, plain call, ambiguous | plain", None),
        ("members: statement, bare, plain call, unique | plain", None),
        ("members: statement, bare, peeled call, unique", Some(true)),
        ("members: statement, bare, plain call, ambiguous | plain", None),
        ("members: statement, bare, peeled call, ambiguous", Some(false)),
        ("members: statement, as, plain call, unique", Some(false)),
        ("members: statement, as, peeled call, unique", Some(false)),
        ("members: statement, as, plain call, ambiguous", Some(false)),
        ("members: statement, as, peeled call, ambiguous", Some(false)),
        ("members: statement, assertion, plain call, unique", Some(true)),
        ("members: statement, assertion, peeled call, unique", Some(true)),
        ("members: statement, assertion, plain call, ambiguous", Some(false)),
        ("members: statement, assertion, peeled call, ambiguous", Some(false)),
        ("members: statement, satisfies, plain call, unique", Some(false)),
        ("members: statement, satisfies, peeled call, unique", Some(false)),
        ("members: statement, satisfies, plain call, ambiguous", Some(false)),
        ("members: statement, satisfies, peeled call, ambiguous", Some(false)),
        ("members: statement, non-null, plain call, unique", Some(false)),
        ("members: statement, non-null, peeled call, unique", Some(false)),
        ("members: statement, non-null, plain call, ambiguous", Some(false)),
        ("members: statement, non-null, peeled call, ambiguous", Some(false)),
        ("members: statement, parens, plain call, unique | plain", None),
        ("members: statement, parens, peeled call, unique", Some(true)),
        ("members: statement, parens, plain call, ambiguous | plain", None),
        ("members: statement, parens, peeled call, ambiguous", Some(false)),
    ] {
        match attaches {
            // Read as origin/main read it: main's own output.
            None => assert_eq!(factory_parity_output(&cases[row]), base[row], "{row}"),
            Some(attaches) => {
                let (plain, peeled) = (format!("{row} | plain"), format!("{row} | peeled"));
                let expected = if attaches { base[plain.as_str()] } else { base[peeled.as_str()] };
                assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
            }
        }
    }
    let row = "members: a namespace-local member behind <T> (finding 2) | plain";
    assert_eq!(factory_parity_output(&cases[row]), base[row], "{row}");
    // The target too: behind `<T>x` in a plain call, it is the parent only as a call main never read trusts it, at module
    // level.
    for (row, attaches) in [
        ("targets: a target behind <T>, at module level", true),
        ("targets: a target behind <T>, in a function", false),
        ("targets: a target behind <T>, bound in a function", false),
        ("targets: a target behind <T>, beside a write to Object.assign", false),
        // An object literal only a peel exposes makes its members peeled.
        ("objects: an object behind <const>, unique", true),
        ("objects: an object behind <const>, ambiguous", false),
    ] {
        let (plain, peeled) = (format!("{row} | plain"), format!("{row} | peeled"));
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if attaches { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
}

/// Round 37: an evidence predicate on a callee written through a cast (origin/main read the callee as written) needs
/// binding evidence: a collection method only on a value known to be a collection, an element helper only when its name
/// means the helper's own declaration. Each plain form is origin/main's; each peeled form reads as the plain form where
/// the evidence holds, and as origin/main read it otherwise. The destructuring of round 36's finding 4 stays as main
/// gives it: no output, written plainly or through `<typeof Ui>`.
#[test]
fn an_evidence_predicate_through_a_cast_needs_binding_evidence() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, holds) in [
        ("evidence: a bare local map (finding 1)", false),
        ("evidence: a map on a value of unknown type", false),
        ("evidence: a map on an array literal", true),
        ("evidence: a helper shadowed by a destructured prop (finding 2)", false),
        ("evidence: a helper declared once", true),
    ] {
        let plain = format!("{row} | plain");
        let peeled = format!("{row} | peeled");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if holds { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
    for form in ["plain", "peeled"] {
        let id = format!("sealed: a destructured import shadowed by a namespace local (finding 4) | {form}");
        assert_eq!(base[id.as_str()], "", "{id}: origin/main");
        assert_eq!(factory_parity_output(&cases[&id]), "", "{id}");
    }
}

/// Round 36: `peel`'s result is sealed (`peeled::Peeled`): a name beneath a wrapper is read only as origin/main read
/// it (`as_plain`) or once its root resolves (`resolve`). A member assigned through an assertion, a mutator written
/// through a cast, a wrapper named by convention, a destructured import and a factory's receiver each follow. Each plain
/// form is origin/main's; each peeled form reads as the plain form where the name resolves, and as origin/main read it
/// otherwise.
#[test]
fn a_sealed_peel_reads_a_name_only_as_main_read_it_or_once_it_resolves() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, resolves) in [
        ("sealed: a member assigned through an assertion, a parameter of its name (finding 1)", false),
        ("sealed: a member assigned through an assertion, declared once", true),
        ("sealed: a wrapper named by convention, a local of its name (finding 3)", false),
        ("sealed: a wrapper named by convention, React's import", true),
        ("sealed: a destructured import shadowed by a namespace local (finding 4)", false),
        ("sealed: a destructured import declared once", true),
        ("sealed: a wrapped mutator writes Object assign (finding 2)", false),
        ("sealed: a new prototype for Object", false),
        ("sealed: a factory on another module's receiver (finding 5)", false),
        ("sealed: a factory on React's receiver", true),
    ] {
        let plain = format!("{row} | plain");
        let peeled = format!("{row} | peeled");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if resolves { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
}

/// Round 34: a name a peeled candidate relies on is a `names::UniqueName`, which only the binding pass makes. A peeled
/// `Object.assign` is recognised only in the minimal shape (a target and one object literal, plain keys written once)
/// and only while the file writes no member that may be `Object.assign`; an identifier alias read only through a
/// `<T>x` assertion only when the identifier is declared once. Each plain form is origin/main's; each peeled form reads
/// as the plain form where trusted, and as origin/main read it otherwise.
#[test]
fn a_peeled_candidate_relies_only_on_names_the_binding_pass_vouches_for() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, trusted) in [
        ("UniqueName: a later source overwrites the member (finding 1)", false),
        ("UniqueName: a spread member", false),
        ("UniqueName: a computed key", false),
        ("UniqueName: a repeated key", false),
        ("UniqueName: Object assign written through a string key (finding 2)", false),
        ("UniqueName: Object assign written through a template key", false),
        ("UniqueName: Object assign written through Reflect.set", false),
        ("UniqueName: Object assign written through defineProperty", false),
        ("UniqueName: Object assign written through defineProperties", false),
        ("UniqueName: Object written through an unknown key", false),
        ("UniqueName: another Object member written", true),
        ("UniqueName: an identifier alias declared once", true),
        ("UniqueName: an identifier alias shadowed by a namespace local (finding 3)", false),
    ] {
        let plain = format!("{row} | plain");
        let peeled = format!("{row} | peeled");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if trusted { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
}

/// Round 33: every well-known entry (`well_known::WELL_KNOWN`) a peeled path matches resolves first: a global must be
/// declared nowhere (an import of its name is another binding), an import must be the name's one declaration and come
/// from the entry's module, aliased or not. Each plain form is origin/main's; each peeled form reads as the plain form
/// where the name resolves, and as origin/main read it otherwise.
#[test]
fn every_well_known_entry_resolves_before_a_peeled_match() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, resolves) in [
        ("well-known table: forwardRef, an import declared once", true),
        ("well-known table: forwardRef, an aliased import", true),
        ("well-known table: forwardRef, an aliased import shadowed by a namespace local (finding 1)", false),
        ("well-known table: forwardRef, an aliased import shadowed by a parameter", false),
        ("well-known table: memo, an import declared once", true),
        ("well-known table: memo, an import shadowed by a parameter", false),
        ("well-known table: createElement, an import declared once", true),
        ("well-known table: createElement, an import shadowed by a parameter", false),
        ("well-known table: h, an import declared once", true),
        ("well-known table: h, an import shadowed by a parameter", false),
        ("well-known table: Promise, an import of its name (finding 2)", false),
        ("well-known table: Promise, a parameter of its name", false),
    ] {
        let plain = format!("{row} | plain");
        let peeled = format!("{row} | peeled");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if resolves { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
}

/// Round 32: an `Object.assign` call's arguments are read in one place (`assign_roles`): a call main never read aliases
/// its result to its first argument only, a target the file declares once, at module level; a later identifier argument
/// is a source, never an alias. And `react_api` behind a `<T>x` assertion (a `.ts` form) matches React only when the
/// receiver resolves to the import. Each plain form is origin/main's; each peeled form reads as the plain form where
/// trusted, and as origin/main read it otherwise.
#[test]
fn an_object_assign_call_is_read_by_its_argument_roles() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, trusted) in [
        ("Object.assign roles, a source declared once (finding 1)", false),
        ("Object.assign roles, a source declared twice", false),
        ("Object.assign roles, an imported source", false),
        ("Object.assign roles, a target declared once", true),
        ("Object.assign roles, a target declared twice (finding 2)", false),
        ("Object.assign roles, a private target declared twice, bound at module level", false),
        ("Object.assign roles, an imported target", true),
        ("Object.assign roles, a default export of a target declared once", true),
        ("Object.assign roles, a default export of a target declared twice", false),
        ("react_api behind a type assertion, an import declared once", true),
        ("react_api behind a type assertion, a parameter of its name", false),
    ] {
        let plain = format!("{row} | plain");
        let peeled = format!("{row} | peeled");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if trusted { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
}

/// Round 31: a peeled path matches a well-known global (`Object`, `Promise`) or import (`Preact`, `React`) by its name only
/// when the name resolves to it: no declaration in the file for a global, the import as its one declaration for an
/// import, and no `Object.assign =` for `Object.assign`. Each plain form is origin/main's; each peeled form reads as the
/// plain form where the name resolves, and as origin/main read it otherwise.
#[test]
fn a_well_known_name_on_a_peeled_path_must_resolve_to_it() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (row, resolves) in [
        ("well-known Preact h, an import declared once", true),
        ("well-known Preact h, a parameter of its name (finding 1)", false),
        ("well-known Preact h, a namespace local of its name", false),
        ("well-known React createElement, an import declared once", true),
        ("well-known React createElement, a parameter of its name", false),
        ("well-known React createElement, a namespace local of its name", false),
        ("well-known Promise, the global", true),
        ("well-known Promise, a local of its name", false),
        ("well-known Object.assign, the global", true),
        ("well-known Object.assign, a local Object (finding 2)", false),
        ("well-known Object.assign, a parameter of its name", false),
        ("well-known Object.assign, assigned in the file", false),
    ] {
        let plain = format!("{row} | plain");
        let peeled = format!("{row} | peeled");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if resolves { base[plain.as_str()] } else { base[peeled.as_str()] };
        assert_eq!(factory_parity_output(&cases[&peeled]), expected, "{peeled}");
    }
}

/// Round 30: origin/main's factory rule read a callee through `as`, `satisfies`, `!`, parentheses and instantiations, so
/// those are `Plain`; only a `<T>x` assertion (a `.ts` file's) is `Peeled`, and then its name must be declared once.
#[test]
fn a_factory_callee_behind_a_type_assertion_is_peeled() {
    let recognition = |callee: &str| {
        let source = format!("declare function f(): unknown;\nconst X = {callee}();");
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &source, oxc_span::SourceType::ts()).parse();
        let Some(Statement::VariableDeclaration(declaration)) = parsed.program.body.get(1) else { panic!("{callee}") };
        let Some(Expression::CallExpression(call)) = declaration.declarations[0].init.as_ref() else {
            panic!("{callee}")
        };
        super::component::Recognition::of_factory_callee(call)
    };
    use super::component::Recognition::{Peeled, Plain};
    for (callee, expected) in [
        ("f", Plain),
        ("(f as typeof f)", Plain),
        ("(f satisfies unknown)", Plain),
        ("f!", Plain),
        ("(f<never>)", Plain),
        ("(<typeof f>f)", Peeled),
    ] {
        assert_eq!(recognition(callee), expected, "{callee}");
    }
    let shadowed = parse_file(
        Utf8Path::new("/fixtures/Button.ts"),
        "import React from 'react';\ninterface Props { label?: string }\n\
         function createButton(): React.FC<Props> { return null as any; }\n\
         function g(createButton: unknown) { return createButton; }\nexport const Card = (<typeof createButton>createButton)();",
    );
    assert!(!shadowed.component_mappings.iter().any(|m| m.component_name == "Card"), "a parameter of its name");
}

/// Round 29: every receiver read through a wrapper, for an imported member alias (`X = (Ui as typeof Ui).Button`), a
/// callee (`(Ui as typeof Ui).wrap<P>(…)`) or a class's heritage (`extends (Ui as typeof Ui).Component`), goes through
/// `receiver_identifier`: read only when the file declares the receiver once, origin/main's answer otherwise. Written
/// plainly, each is origin/main's.
#[test]
fn every_receiver_read_through_a_cast_is_read_only_when_declared_once() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    let now = |id: &str| factory_parity_output(&cases[id]);
    for consumer in ["alias", "callee", "heritage"] {
        for shadow in
            ["declared once", "shadowed by a namespace local", "shadowed by a parameter", "imported and declared"]
        {
            let plain = format!("receiver of a {consumer}, {shadow} | plain");
            let cast = format!("receiver of a {consumer}, {shadow} | typeof itself");
            assert_eq!(now(&plain), base[plain.as_str()], "{plain}");
            // Declared once, the cast reads as the plain form; otherwise as origin/main read the cast.
            let expected = if shadow == "declared once" { base[plain.as_str()] } else { base[cast.as_str()] };
            assert_eq!(now(&cast), expected, "{cast}");
        }
    }
    let finding = "finding: a namespace-local receiver read through a cast";
    assert_eq!(now(finding), base[finding]);
    assert!(!base[finding].contains("PublicButton:"), "{}", base[finding]);
}

/// Round 27: export propagation finds an `Object.assign`'s parent by its bare name, so one written through a cast,
/// which origin/main never read, attaches its members only at module level (a statement there, an exported
/// declaration's initializer or the default export) and only when the file declares the parent exactly once. Written
/// plainly, it is origin/main's wherever it is, a shadowing local included.
#[test]
fn an_object_assign_through_a_cast_attaches_only_at_module_level_to_a_parent_declared_once() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    for (form, attaches) in [
        ("a shadowing local Root in a function", false),
        ("at module level", true),
        ("in a nested function", false),
        ("in an arrow initializer", false),
        ("in a class method", false),
        ("as the default export", true),
        ("as an exported declaration's initializer", true),
        ("with a second module-level Root", false),
    ] {
        let plain = format!("Object.assign {form} | plain");
        let cast = format!("Object.assign {form} | typeof itself");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        let expected = if attaches { base[plain.as_str()] } else { base[cast.as_str()] };
        assert_eq!(factory_parity_output(&cases[&cast]), expected, "{cast}");
    }
    // The finding: written plainly main attaches the shadowing local's member to the public `Root`; through the cast
    // main attached nothing, and neither does this head.
    assert!(base["Object.assign a shadowing local Root in a function | plain"].contains("exported as Root.Item"));
    assert!(!base["Object.assign a shadowing local Root in a function | typeof itself"].contains("Root.Item"));
}

/// Round 26: a callee written through a cast is looked up by its bare name (`resolver::callee`), so it is attached only
/// when the file declares that name exactly once, by the same flat count as the `typeof` aliases. Otherwise it is
/// unset, as on origin/main: the component is found and the callee's declaration is not read. A plain call is
/// origin/main's whatever the count.
#[test]
fn a_callee_through_a_cast_is_attached_only_when_its_name_is_declared_once() {
    let base: FxHashMap<&str, &str> =
        include_str!("factory_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases: FxHashMap<String, String> = factory_parity_cases().into_iter().collect();
    let attached = "Base:Props\"functionDeclaration\" | Button:Props\"hocWrapper\" exported as Button via wrap";
    let unset = "Base:Props\"functionDeclaration\" | Button:Props\"hocWrapper\" exported as Button";
    for (row, cast) in [
        ("callee declared once", attached),
        ("callee declared twice at module level", unset),
        ("callee declared again in a namespace", unset),
        ("callee name a parameter elsewhere", unset),
        ("callee imported and declared", unset),
    ] {
        let plain = format!("{row} | plain");
        let through = format!("{row} | typeof itself");
        assert_eq!(factory_parity_output(&cases[&plain]), base[plain.as_str()], "{plain}");
        assert_eq!(base[plain.as_str()], attached, "{plain}: origin/main");
        assert_eq!(base[through.as_str()], unset, "{through}: origin/main");
        assert_eq!(factory_parity_output(&cases[&through]), cast, "{through}");
    }
    let finding = "finding: a namespace's own wrap called through typeof itself";
    assert_eq!(factory_parity_output(&cases[finding]), base[finding]);
    assert!(!base[finding].contains(" via "), "{}", base[finding]);
}

/// Finding 1 of round 9, and round 10's one walker: every `typeof` alias's declarations are counted in one walk of the
/// module, however many aliases there are; a module with none is walked once too, for its callees (round 26).
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
        let kept =
            super::component::typeof_aliases(&parsed.program, &super::component::file_names(&parsed.program)).len();
        (kept, super::component::steps::take_walks())
    };
    assert_eq!(cost(0), (0, 1));
    assert_eq!(cost(1), (1, 1));
    assert_eq!(cost(10), (10, 1));
    assert_eq!(cost(40), (40, 1));
}

/// Review round 25, the flat rule: a `typeof` alias is read as the callee's own type only when the file declares its
/// name once in type space. Any second declaration, in any scope and of any kind, makes the cast `Unknown`: the
/// component is kept and the wrapper's own declaration, whose result adds `internal`, is not read.
#[test]
fn a_typeof_alias_is_read_only_when_its_name_is_declared_once_in_the_file() {
    let wrap = "function wrap<T>(render: React.FC<T>): React.FC<T & { internal?: string }> { return render as any; }\n\
                type Wrap = typeof wrap;\n";
    let build = |other: &str, callee: &str| {
        tsx(&format!("{wrap}{other}\nexport const Button = {callee}<Props>((props: Props) => <div />);"))
    };
    let plain = build("", "wrap");
    let replaced = build("", "(wrap as <T>(render: React.FC<T>) => React.FC<T>)");
    assert_eq!(plain.len(), 1, "{plain:#?}");
    assert_ne!(plain, replaced);
    assert_eq!(build("", "(wrap as Wrap)"), plain, "declared once: the callee's own type");
    for (kind, other) in [
        ("a type parameter elsewhere", "function g<Wrap>(w: Wrap) { return w; }"),
        ("a nested namespace's member", "namespace N { namespace M { export type Wrap = () => void; } }"),
        ("merged namespace blocks", "namespace N { export type Wrap = () => void; }\nnamespace N { const x = 1; }"),
        (
            "round 25 finding: a private nested namespace in each of two blocks",
            "namespace N { namespace M { export type Wrap = () => void; } }\n\
             namespace N { namespace M { export const B = (wrap as Wrap)<Props>((props: Props) => <div />); } }",
        ),
        ("an import", "import type { Wrap } from './wrap';"),
        ("an interface merged with it", "interface Wrap { (): void }"),
        ("a mapped key", "type Keys = { [Wrap in 'a']: Wrap };"),
        ("an infer", "type First<T> = T extends Array<infer Wrap> ? Wrap : never;"),
        ("a class expression's name", "const K = class Wrap {};"),
        ("a declare module member", "declare module 'x' { type Wrap = () => void; }"),
    ] {
        let unknown = build(other, "(wrap as Wrap)");
        let expected = build(other, "(wrap as <T>(render: React.FC<T>) => React.FC<T>)");
        assert_eq!(unknown, expected, "{kind}");
        assert_ne!(unknown, build(other, "(wrap as typeof wrap)"), "{kind}");
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

/// Finding 2, and round 26: every module is walked once for the names it declares, whether it has a `typeof` alias or
/// not, as a callee written through a cast needs the count too.
#[test]
fn every_module_is_walked_once_for_the_names_it_declares() {
    let walks = |source: &str| {
        super::component::steps::take_walks();
        parse_file(Utf8Path::new("/fixtures/Button.tsx"), &format!("{HEADER}{source}"));
        super::component::steps::take_walks()
    };
    let generic = "function g<Factory>(f: Factory) { return f; }\ntype Mapped = { [K in 'a']: K };\n\
                   export const Button = memo((props: Props) => <div />);";
    assert_eq!(walks(generic), 1);
    assert_eq!(walks(&format!("type Size = typeof size;\ndeclare const size: number;\n{generic}")), 1);
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

/// Round 13, finding 2: a `typeof` alias of a long dotted chain is named in one pass over its segments.
#[test]
fn a_typeof_alias_of_a_long_chain_is_named_in_linear_steps() {
    let written = |segments: usize| {
        let chain: Vec<String> = (0..segments).map(|i| format!("N{i}")).collect();
        let source = format!("declare const N0: any;\ntype Factory = typeof {};\n", chain.join("."));
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &source, oxc_span::SourceType::tsx()).parse();
        super::component::steps::take_written();
        let aliases = super::component::typeof_aliases(&parsed.program, &super::component::file_names(&parsed.program));
        assert_eq!(aliases.get("Factory").map(|alias| alias.chain.as_str()), Some(chain.join(".").as_str()));
        (super::component::steps::take_written(), chain.join(".").len() as u64 + 1)
    };
    for segments in [2, 20, 200] {
        let (bytes, length) = written(segments);
        assert_eq!(bytes, length, "{segments} segments");
    }
}
