//! Every component detection form, crossed with every wrapper TypeScript can put around the expression it reads,
//! extracts the same component as the unwrapped form. OXC 0.159 stopped flattening `(f<T>)(x)` and
//! `extends (B<T>)` into the enclosing call or heritage clause, which is how a wrapper first broke detection.

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
}

impl Callee {
    const ALL: [Self; 13] = [
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
    ];

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
            let (expected, errors) = extracted(&path, &(form.source)(Callee::Plain, Value::Plain));
            assert!(errors.is_empty(), "{}: {errors:?}", form.name);
            let [mapping] = expected.as_slice() else {
                panic!("{}: expected one component, got {expected:#?}", form.name)
            };
            assert_eq!(mapping["component_name"], "Button", "{}", form.name);
            assert_eq!(mapping["props_type_name"], "Props", "{}", form.name);
            let rows = Callee::ALL
                .iter()
                .map(|&callee| (callee, Value::Plain))
                .chain(Value::ALL.iter().map(|&value| (Callee::Plain, value)));
            for (callee, value) in rows {
                let jsx_only = extension == "tsx";
                if jsx_only && (matches!(callee, Callee::TypeAssertion) || matches!(value, Value::TypeAssertion)) {
                    continue;
                }
                let source = (form.source)(callee, value);
                let (actual, errors) = extracted(&path, &source);
                if !errors.is_empty() {
                    failures.push(format!(
                        "{} [{callee:?}, {value:?}] .{extension}: parse errors {errors:?}\n{source}",
                        form.name
                    ));
                } else if actual != expected {
                    failures.push(format!(
                        "{} [{callee:?}, {value:?}] .{extension}:\n{source}\nexpected {expected:#?}\nactual {actual:#?}",
                        form.name
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} rows differ from the unwrapped form:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
