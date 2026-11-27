//! OXC 0.159 parity: every row reads as origin/main (OXC 0.158) read it. The rows are printed on origin/main into
//! `oxc_parity_base.tsv` and compared here.
//!
//! - An instantiation around a callee, `(f<A>)(…)`, is what OXC 0.158 parsed as type arguments on the call, so it is read
//!   as the bare callee (`component::unwrap_transparent`), like parentheses.
//! - A cast, `satisfies` or `!` around a callee, receiver, argument, initializer or class base is read as main read it.

use rustc_hash::FxHashMap;

// BEGIN FACTORY PARITY CASES.
use camino::Utf8Path;

use super::parse_file;

fn factory_parity_cases() -> Vec<(String, String)> {
    [
        (
            "instantiation: forwardRef, a bare callee",
            "import { forwardRef } from 'react';\nexport const Button = (forwardRef<HTMLButtonElement, Props>)((props, ref) => <div />);",
        ),
        (
            "instantiation: forwardRef, a member chain",
            "import * as R from 'react';\nexport const Button = (R.forwardRef<HTMLButtonElement, Props>)((props, ref) => <div />);",
        ),
        (
            "instantiation: memo, a bare callee",
            "import { memo } from 'react';\nexport const Button = (memo<Props>)((props: Props) => <div />);",
        ),
        (
            "instantiation: memo, a member chain",
            "import * as React from 'react';\nexport const Button = (React.memo<Props>)((props: Props) => <div />);",
        ),
        (
            "instantiation: lazy, a bare callee",
            "import { lazy } from 'react';\nexport const Button = (lazy<typeof Impl>)(() => import('./Impl'));",
        ),
        (
            "instantiation: createElement, a bare callee",
            "import { createElement } from 'react';\nexport const Button = (props: Props) => (createElement<Props>)('div');",
        ),
        (
            "instantiation: createElement, a member chain",
            "import * as React from 'react';\nexport const Button = (props: Props) => (React.createElement<Props>)('div');",
        ),
        (
            "instantiation: h, a bare callee",
            "import { h } from 'preact';\nexport const Button = (props: Props) => (h<Props>)('div');",
        ),
        (
            "instantiation: cloneElement, a bare callee",
            "import { cloneElement } from 'react';\nexport const Button = (props: Props) => (cloneElement<Props>)(<div />);",
        ),
        (
            "instantiation: Object.assign, an inline member (finding 1)",
            "interface ItemProps { tone?: string }\ntype Members = {};\nexport function Root(props: Props) { return <div />; }\n(Object.assign<typeof Root, Members>)(Root, { Item: (p: ItemProps) => <span /> });",
        ),
        (
            "instantiation: Object.assign, a declarator",
            "interface ItemProps { tone?: string }\nfunction Impl(props: Props) { return <div />; }\nexport const Root = (Object.assign<typeof Impl, {}>)(Impl, { Item: (p: ItemProps) => <span /> });",
        ),
        (
            "instantiation: Object.assign, a default export",
            "function Impl(props: Props) { return <div />; }\nexport default (Object.assign<typeof Impl, {}>)(Impl, {});",
        ),
        (
            "instantiation: Promise.resolve",
            "export const Button = (props: Props) => (Promise.resolve<JSX.Element>)(<div />);",
        ),
        (
            "instantiation: Array.isArray",
            "export const Button = (props: Props) => (Array.isArray<unknown>)(props) ? null : <div />;",
        ),
        (
            "instantiation: a map",
            "interface ListProps { items: string[] }\nexport const Button = (props: ListProps) => (props.items.map<JSX.Element>)((i) => <div />);",
        ),
        (
            "instantiation: a helper (finding 2)",
            "import * as React from 'react';\nfunction makeElement<P>(): React.ReactElement { return <div />; }\nexport const Button = (props: Props) => { return (makeElement<Props>)(); };",
        ),
        (
            "instantiation: a factory call",
            "import * as React from 'react';\ndeclare function createButton<P>(): React.FC<P>;\nexport const Button = (createButton<Props>)();",
        ),
        (
            "instantiation: a hook",
            "declare function useLabel<T>(): void;\nexport const Label = (props: Props) => { (useLabel<Props>)(); return props.label; };",
        ),
        (
            "instantiation: a PascalCase call",
            "function Inner(props: Props) { return <div />; }\nexport const Button = (props: Props) => (Inner<Props>)(props);",
        ),
        (
            "instantiation: a wrapper",
            "import * as React from 'react';\ndeclare function withTheme<P>(c: unknown): React.FC<P>;\nfunction Base(props: Props) { return <div />; }\nexport const Button = (withTheme<Props>)(Base);",
        ),
        (
            "instantiation: a class base",
            "import { Component } from 'react';\nexport class Button extends (Component<Props>) { render() { return <div />; } }",
        ),
        (
            "instantiation: a member class base",
            "import * as React from 'react';\nexport class Button extends (React.Component<Props>) { render() { return <div />; } }",
        ),
        (
            "instantiation: a tag",
            "function Foo<T>(p: T) { return <div />; }\nexport const Button = (props: Props) => <Foo<Props> {...props} />;",
        ),
        (
            "instantiation: a default export",
            "import { memo } from 'react';\nfunction Impl(props: Props) { return <div />; }\nexport default (memo<Props>)(Impl);",
        ),
        (
            "instantiation: an argument",
            "import { memo } from 'react';\nfunction Impl<T>(props: Props) { return <div />; }\nexport const Button = memo(Impl<Props>);",
        ),
        (
            "instantiation: a tagged template",
            "import styled from 'styled-components';\nexport const Button = (styled.button<Props>)`color: red;`;",
        ),
        (
            "instantiation: a new expression",
            "class Box<T> { value?: T }\nexport const Button = (props: Props) => { new (Box<Props>)(); return <div />; };",
        ),
        (
            "retreat: plain, callee",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: plain, receiver",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = React.forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: plain, argument",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = memo(Impl);",
        ),
        (
            "retreat: plain, initializer",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = memo(Impl);",
        ),
        (
            "retreat: plain, class base",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport class Button extends Component<Props> { render() { return createElement('div'); } }",
        ),
        (
            "retreat: as, callee",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (forwardRef as typeof forwardRef)<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: as, receiver",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (React as typeof React).forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: as, argument",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = memo((Impl as typeof Impl));",
        ),
        (
            "retreat: as, initializer",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (memo(Impl)) as any;",
        ),
        (
            "retreat: as, class base",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport class Button extends (Component as typeof Component)<Props> { render() { return createElement('div'); } }",
        ),
        (
            "retreat: assertion, callee",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (<typeof forwardRef>forwardRef)<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: assertion, receiver",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (<typeof React>React).forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: assertion, argument",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = memo((<typeof Impl>Impl));",
        ),
        (
            "retreat: assertion, initializer",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = <any>(memo(Impl));",
        ),
        (
            "retreat: assertion, class base",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport class Button extends (<typeof Component>Component)<Props> { render() { return createElement('div'); } }",
        ),
        (
            "retreat: satisfies, callee",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (forwardRef satisfies typeof forwardRef)<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: satisfies, receiver",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (React satisfies typeof React).forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: satisfies, argument",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = memo((Impl satisfies typeof Impl));",
        ),
        (
            "retreat: satisfies, initializer",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (memo(Impl)) satisfies unknown;",
        ),
        (
            "retreat: satisfies, class base",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport class Button extends (Component satisfies typeof Component)<Props> { render() { return createElement('div'); } }",
        ),
        (
            "retreat: non-null, callee",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = forwardRef!<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: non-null, receiver",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = React!.forwardRef<HTMLButtonElement, Props>((props, ref) => createElement('div'));",
        ),
        (
            "retreat: non-null, argument",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = memo(Impl!);",
        ),
        (
            "retreat: non-null, initializer",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport const Button = (memo(Impl))!;",
        ),
        (
            "retreat: non-null, class base",
            "// ts\nimport { Component, createElement, forwardRef, memo } from 'react';\nimport * as React from 'react';\nfunction Impl(props: Props) { return createElement('div'); }\nexport class Button extends Component!<Props> { render() { return createElement('div'); } }",
        ),
    ]
    .into_iter()
    .map(|(id, source)| (id.to_owned(), source.to_owned()))
    .collect()
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

#[test]
fn every_row_reads_as_origin_main_read_it() {
    let base: FxHashMap<&str, &str> =
        include_str!("oxc_parity_base.tsv").lines().filter_map(|line| line.split_once('\t')).collect();
    let cases = factory_parity_cases();
    assert_eq!(base.len(), cases.len(), "the base table and the cases are out of step");
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(id, source)| {
            let now = factory_parity_output(source);
            let main = base.get(id.as_str()).copied().unwrap_or("not in the base table");
            (now != main).then(|| format!("{id}:\n  main {main}\n  now  {now}"))
        })
        .collect();
    assert!(failures.is_empty(), "{} rows:\n{}", failures.len(), failures.join("\n"));
}

/// The flatten walk enters no type: the 600-level chained conditional type of
/// `deeply_chained_conditional_types_hit_the_depth_guard_not_the_bracket_heuristic` costs it nothing, and the alias
/// collector's own depth limit stays the only walk over it.
#[test]
fn the_flatten_walk_enters_no_type() {
    let mut ty = "boolean".to_owned();
    for index in 0..600 {
        ty = format!("T{index} extends string ? {ty} : never");
    }
    let source = format!("type Deep = {ty};\nexport const Button = (forwardRef<HTMLButtonElement, Deep>)(() => null);");
    super::flatten_probes::take();
    parse_file(Utf8Path::new("/test/deep-conditional.ts"), &source);
    assert_eq!(super::flatten_probes::take(), 0);
}
