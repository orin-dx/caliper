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
    /// `(React as object).forwardRef`: the namespace is asserted to a type with no such member.
    ObjectAsObject,
    /// `(<(...args: any[]) => object>f)`: the `<T>x` spelling of `AsObjectResult`, only valid where JSX is not.
    TypeAssertionObjectResult,
}

/// What a row extracts, against the unwrapped form.
#[derive(Debug, PartialEq)]
enum Expect {
    /// The assertion keeps the callee's type (none, `typeof` itself, `any`): the unwrapped form's component.
    Same,
    /// The assertion replaces the callee's type with a named result that is not resolved: the name still says what the
    /// call is, but the callee's own declaration no longer describes it, so the mapping has no wrapper callee.
    SameWithoutCallee,
    /// The assertion states a result that is no named type: nothing is read through the callee's name.
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
            Self::AsOtherComponentResult => Expect::SameWithoutCallee,
            Self::AsObjectResult
            | Self::AsUnknownAsObjectResult
            | Self::ObjectAsObject
            | Self::TypeAssertionObjectResult => Expect::Declined,
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
                        "{} [{callee:?}, {value:?}] .{extension}, {expect:?}:\n{source}\nexpected {expected:#?}\nactual {actual:#?}",
                        form.name
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} rows differ from what they expect:\n\n{}", failures.len(), failures.join("\n\n"));
}

/// `Object.assign` through an identity cast makes `Root` an alias of `Impl`, and the inline member scan reads the call
/// through the same predicate, so `Item` is found with it. A cast of `Object` to another type finds neither.
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
    for object in ["Object", "(Object as typeof Object)", "(Object as any)", "(Object)!"] {
        assert_eq!(found(object), [pair("Item", "ItemProps"), pair("Root", "P")], "{object}");
    }
    // Not an alias, `Root` takes nothing and `Impl` stands as its own component.
    assert_eq!(found("(Object as object)"), [pair("Impl", "P")]);
}

/// A PascalCase callee is rendering evidence only while its name describes the call: an assertion stating a result
/// that is no named type takes it away; a named or React result, `typeof` itself and `any` leave it.
#[test]
fn an_asserted_callee_renders_only_when_the_assertion_leaves_a_named_result() {
    let rows = [
        ("(Object as (value: object) => object)", false),
        ("(Object as unknown as (value: object) => string)", false),
        ("(Object as { (value: object): object })", false),
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
