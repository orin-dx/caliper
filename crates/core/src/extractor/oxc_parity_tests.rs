//! OXC 0.159 parity: every row reads as origin/main (OXC 0.158) read it. The rows are printed on origin/main into
//! `oxc_parity_base.tsv` and compared here.
//!
//! - An instantiation around a callee, `(f<A>)(…)`, is what OXC 0.158 parsed as type arguments on the call, so it is read
//!   as the bare callee with the call's type arguments (`component::callee`, `call_type_arguments`, `heritage_base`), except
//!   on an optional call, where 0.158 kept it too.
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
            "instantiation: forwardRef inside memo",
            "import { forwardRef, memo } from 'react';\nexport const Button = memo((forwardRef<HTMLButtonElement, Props>)((props, ref) => <div />));",
        ),
        (
            "instantiation: another module's forwardRef",
            "import { forwardRef } from 'react-aria';\nexport const Button = (forwardRef<HTMLButtonElement, Props>)((props, ref) => <div />);",
        ),
        (
            "instantiation: a class base with an inline props type",
            "import * as React from 'react';\nexport class Button extends (React.Component<{ a: string }>) { render() { return <div />; } }",
        ),
        (
            "instantiation: a component rendered through memo and called",
            "import { memo } from 'react';\nfunction Inner(props: Props) { return <div />; }\nexport const Wrapped = (memo<Props>)(Inner);\nexport const Button = (props: Props) => Inner(props);",
        ),
        (
            "instantiation: a component rendered through forwardRef and called",
            "import { forwardRef } from 'react';\nfunction Inner(props: Props) { return <div />; }\nexport const Wrapped = (forwardRef<HTMLElement, Props>)(Inner);\nexport const Button = (props: Props) => Inner(props);",
        ),
        (
            "instantiation: a wrapper by convention around a component",
            "import * as React from 'react';\ndeclare function withTheme<P>(c: unknown): React.FC<P>;\nfunction Base(props: Props) { return <div />; }\nexport const Button = (withTheme<Props>)(Base);\nexport default (withTheme<Props>)(Base);",
        ),
        (
            "instantiation: a class base with a props type no rule reads",
            "import * as React from 'react';\nexport class Button extends (React.Component<keyof Props>) { render() { return <div />; } }",
        ),
        (
            "normalisation: a function-local enum with a long chain (finding 1)",
            "// ts\nexport function f() { enum E { A = 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 } return E.A; }",
        ),
        (
            "normalisation: a module enum with a long chain",
            "// ts\nexport enum E { A = 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 }",
        ),
        (
            "normalisation: an expression statement with a long chain",
            "// ts\nlet a = 0;\na = 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1;",
        ),
        (
            "normalisation: a type alias with a long union",
            "// ts\nexport type U = 'v0' | 'v1' | 'v2' | 'v3' | 'v4' | 'v5' | 'v6' | 'v7' | 'v8' | 'v9' | 'v10' | 'v11' | 'v12' | 'v13' | 'v14' | 'v15' | 'v16' | 'v17' | 'v18' | 'v19' | 'v20' | 'v21' | 'v22' | 'v23' | 'v24' | 'v25' | 'v26' | 'v27' | 'v28' | 'v29' | 'v30' | 'v31' | 'v32' | 'v33' | 'v34' | 'v35' | 'v36' | 'v37' | 'v38' | 'v39' | 'v40' | 'v41' | 'v42' | 'v43' | 'v44' | 'v45' | 'v46' | 'v47' | 'v48' | 'v49' | 'v50' | 'v51' | 'v52' | 'v53' | 'v54' | 'v55' | 'v56' | 'v57' | 'v58' | 'v59' | 'v60' | 'v61' | 'v62' | 'v63' | 'v64' | 'v65' | 'v66' | 'v67' | 'v68' | 'v69' | 'v70' | 'v71' | 'v72' | 'v73' | 'v74' | 'v75' | 'v76' | 'v77' | 'v78' | 'v79' | 'v80' | 'v81' | 'v82' | 'v83' | 'v84' | 'v85' | 'v86' | 'v87' | 'v88' | 'v89' | 'v90' | 'v91' | 'v92' | 'v93' | 'v94' | 'v95' | 'v96' | 'v97' | 'v98' | 'v99' | 'v100' | 'v101' | 'v102' | 'v103' | 'v104' | 'v105' | 'v106' | 'v107' | 'v108' | 'v109' | 'v110' | 'v111' | 'v112' | 'v113' | 'v114' | 'v115' | 'v116' | 'v117' | 'v118' | 'v119' | 'v120' | 'v121' | 'v122' | 'v123' | 'v124' | 'v125' | 'v126' | 'v127' | 'v128' | 'v129' | 'v130' | 'v131' | 'v132' | 'v133' | 'v134' | 'v135' | 'v136' | 'v137' | 'v138' | 'v139' | 'v140' | 'v141' | 'v142' | 'v143' | 'v144' | 'v145' | 'v146' | 'v147' | 'v148' | 'v149' | 'v150' | 'v151' | 'v152' | 'v153' | 'v154' | 'v155' | 'v156' | 'v157' | 'v158' | 'v159' | 'v160' | 'v161' | 'v162' | 'v163' | 'v164' | 'v165' | 'v166' | 'v167' | 'v168' | 'v169' | 'v170' | 'v171' | 'v172' | 'v173' | 'v174' | 'v175' | 'v176' | 'v177' | 'v178' | 'v179' | 'v180' | 'v181' | 'v182' | 'v183' | 'v184' | 'v185' | 'v186' | 'v187' | 'v188' | 'v189' | 'v190' | 'v191' | 'v192' | 'v193' | 'v194' | 'v195' | 'v196' | 'v197' | 'v198' | 'v199' | 'v200' | 'v201' | 'v202' | 'v203' | 'v204' | 'v205' | 'v206' | 'v207' | 'v208' | 'v209' | 'v210' | 'v211' | 'v212' | 'v213' | 'v214' | 'v215' | 'v216' | 'v217' | 'v218' | 'v219' | 'v220' | 'v221' | 'v222' | 'v223' | 'v224' | 'v225' | 'v226' | 'v227' | 'v228' | 'v229' | 'v230' | 'v231' | 'v232' | 'v233' | 'v234' | 'v235' | 'v236' | 'v237' | 'v238' | 'v239' | 'v240' | 'v241' | 'v242' | 'v243' | 'v244' | 'v245' | 'v246' | 'v247' | 'v248' | 'v249' | 'v250' | 'v251' | 'v252' | 'v253' | 'v254' | 'v255' | 'v256' | 'v257' | 'v258' | 'v259' | 'v260' | 'v261' | 'v262' | 'v263' | 'v264' | 'v265' | 'v266' | 'v267' | 'v268' | 'v269' | 'v270' | 'v271' | 'v272' | 'v273' | 'v274' | 'v275' | 'v276' | 'v277' | 'v278' | 'v279' | 'v280' | 'v281' | 'v282' | 'v283' | 'v284' | 'v285' | 'v286' | 'v287' | 'v288' | 'v289' | 'v290' | 'v291' | 'v292' | 'v293' | 'v294' | 'v295' | 'v296' | 'v297' | 'v298' | 'v299' | 'v300' | 'v301' | 'v302' | 'v303' | 'v304' | 'v305' | 'v306' | 'v307' | 'v308' | 'v309' | 'v310' | 'v311' | 'v312' | 'v313' | 'v314' | 'v315' | 'v316' | 'v317' | 'v318' | 'v319' | 'v320' | 'v321' | 'v322' | 'v323' | 'v324' | 'v325' | 'v326' | 'v327' | 'v328' | 'v329' | 'v330' | 'v331' | 'v332' | 'v333' | 'v334' | 'v335' | 'v336' | 'v337' | 'v338' | 'v339' | 'v340' | 'v341' | 'v342' | 'v343' | 'v344' | 'v345' | 'v346' | 'v347' | 'v348' | 'v349' | 'v350' | 'v351' | 'v352' | 'v353' | 'v354' | 'v355' | 'v356' | 'v357' | 'v358' | 'v359' | 'v360' | 'v361' | 'v362' | 'v363' | 'v364' | 'v365' | 'v366' | 'v367' | 'v368' | 'v369' | 'v370' | 'v371' | 'v372' | 'v373' | 'v374' | 'v375' | 'v376' | 'v377' | 'v378' | 'v379' | 'v380' | 'v381' | 'v382' | 'v383' | 'v384' | 'v385' | 'v386' | 'v387' | 'v388' | 'v389' | 'v390' | 'v391' | 'v392' | 'v393' | 'v394' | 'v395' | 'v396' | 'v397' | 'v398' | 'v399' | 'v400' | 'v401' | 'v402' | 'v403' | 'v404' | 'v405' | 'v406' | 'v407' | 'v408' | 'v409' | 'v410' | 'v411' | 'v412' | 'v413' | 'v414' | 'v415' | 'v416' | 'v417' | 'v418' | 'v419' | 'v420' | 'v421' | 'v422' | 'v423' | 'v424' | 'v425' | 'v426' | 'v427' | 'v428' | 'v429' | 'v430' | 'v431' | 'v432' | 'v433' | 'v434' | 'v435' | 'v436' | 'v437' | 'v438' | 'v439' | 'v440' | 'v441' | 'v442' | 'v443' | 'v444' | 'v445' | 'v446' | 'v447' | 'v448' | 'v449' | 'v450' | 'v451' | 'v452' | 'v453' | 'v454' | 'v455' | 'v456' | 'v457' | 'v458' | 'v459' | 'v460' | 'v461' | 'v462' | 'v463' | 'v464' | 'v465' | 'v466' | 'v467' | 'v468' | 'v469' | 'v470' | 'v471' | 'v472' | 'v473' | 'v474' | 'v475' | 'v476' | 'v477' | 'v478' | 'v479' | 'v480' | 'v481' | 'v482' | 'v483' | 'v484' | 'v485' | 'v486' | 'v487' | 'v488' | 'v489' | 'v490' | 'v491' | 'v492' | 'v493' | 'v494' | 'v495' | 'v496' | 'v497' | 'v498' | 'v499' | 'v500' | 'v501' | 'v502' | 'v503' | 'v504' | 'v505' | 'v506' | 'v507' | 'v508' | 'v509' | 'v510' | 'v511' | 'v512' | 'v513' | 'v514' | 'v515' | 'v516' | 'v517' | 'v518' | 'v519' | 'v520' | 'v521' | 'v522' | 'v523' | 'v524' | 'v525' | 'v526' | 'v527' | 'v528' | 'v529' | 'v530' | 'v531' | 'v532' | 'v533' | 'v534' | 'v535' | 'v536' | 'v537' | 'v538' | 'v539' | 'v540' | 'v541' | 'v542' | 'v543' | 'v544' | 'v545' | 'v546' | 'v547' | 'v548' | 'v549' | 'v550' | 'v551' | 'v552' | 'v553' | 'v554' | 'v555' | 'v556' | 'v557' | 'v558' | 'v559' | 'v560' | 'v561' | 'v562' | 'v563' | 'v564' | 'v565' | 'v566' | 'v567' | 'v568' | 'v569' | 'v570' | 'v571' | 'v572' | 'v573' | 'v574' | 'v575' | 'v576' | 'v577' | 'v578' | 'v579' | 'v580' | 'v581' | 'v582' | 'v583' | 'v584' | 'v585' | 'v586' | 'v587' | 'v588' | 'v589' | 'v590' | 'v591' | 'v592' | 'v593' | 'v594' | 'v595' | 'v596' | 'v597' | 'v598' | 'v599' | 'v600' | 'v601' | 'v602' | 'v603' | 'v604' | 'v605' | 'v606' | 'v607' | 'v608' | 'v609' | 'v610' | 'v611' | 'v612' | 'v613' | 'v614' | 'v615' | 'v616' | 'v617' | 'v618' | 'v619' | 'v620' | 'v621' | 'v622' | 'v623' | 'v624' | 'v625' | 'v626' | 'v627' | 'v628' | 'v629' | 'v630' | 'v631' | 'v632' | 'v633' | 'v634' | 'v635' | 'v636' | 'v637' | 'v638' | 'v639' | 'v640' | 'v641' | 'v642' | 'v643' | 'v644' | 'v645' | 'v646' | 'v647' | 'v648' | 'v649' | 'v650' | 'v651' | 'v652' | 'v653' | 'v654' | 'v655' | 'v656' | 'v657' | 'v658' | 'v659' | 'v660' | 'v661' | 'v662' | 'v663' | 'v664' | 'v665' | 'v666' | 'v667' | 'v668' | 'v669' | 'v670' | 'v671' | 'v672' | 'v673' | 'v674' | 'v675' | 'v676' | 'v677' | 'v678' | 'v679' | 'v680' | 'v681' | 'v682' | 'v683' | 'v684' | 'v685' | 'v686' | 'v687' | 'v688' | 'v689' | 'v690' | 'v691' | 'v692' | 'v693' | 'v694' | 'v695' | 'v696' | 'v697' | 'v698' | 'v699' | 'v700' | 'v701' | 'v702' | 'v703' | 'v704' | 'v705' | 'v706' | 'v707' | 'v708' | 'v709' | 'v710' | 'v711' | 'v712' | 'v713' | 'v714' | 'v715' | 'v716' | 'v717' | 'v718' | 'v719' | 'v720' | 'v721' | 'v722' | 'v723' | 'v724' | 'v725' | 'v726' | 'v727' | 'v728' | 'v729' | 'v730' | 'v731' | 'v732' | 'v733' | 'v734' | 'v735' | 'v736' | 'v737' | 'v738' | 'v739' | 'v740' | 'v741' | 'v742' | 'v743' | 'v744' | 'v745' | 'v746' | 'v747' | 'v748' | 'v749' | 'v750' | 'v751' | 'v752' | 'v753' | 'v754' | 'v755' | 'v756' | 'v757' | 'v758' | 'v759' | 'v760' | 'v761' | 'v762' | 'v763' | 'v764' | 'v765' | 'v766' | 'v767' | 'v768' | 'v769' | 'v770' | 'v771' | 'v772' | 'v773' | 'v774' | 'v775' | 'v776' | 'v777' | 'v778' | 'v779' | 'v780' | 'v781' | 'v782' | 'v783' | 'v784' | 'v785' | 'v786' | 'v787' | 'v788' | 'v789' | 'v790' | 'v791' | 'v792' | 'v793' | 'v794' | 'v795' | 'v796' | 'v797' | 'v798' | 'v799' | 'v800' | 'v801' | 'v802' | 'v803' | 'v804' | 'v805' | 'v806' | 'v807' | 'v808' | 'v809' | 'v810' | 'v811' | 'v812' | 'v813' | 'v814' | 'v815' | 'v816' | 'v817' | 'v818' | 'v819' | 'v820' | 'v821' | 'v822' | 'v823' | 'v824' | 'v825' | 'v826' | 'v827' | 'v828' | 'v829' | 'v830' | 'v831' | 'v832' | 'v833' | 'v834' | 'v835' | 'v836' | 'v837' | 'v838' | 'v839' | 'v840' | 'v841' | 'v842' | 'v843' | 'v844' | 'v845' | 'v846' | 'v847' | 'v848' | 'v849' | 'v850' | 'v851' | 'v852' | 'v853' | 'v854' | 'v855' | 'v856' | 'v857' | 'v858' | 'v859' | 'v860' | 'v861' | 'v862' | 'v863' | 'v864' | 'v865' | 'v866' | 'v867' | 'v868' | 'v869' | 'v870' | 'v871' | 'v872' | 'v873' | 'v874' | 'v875' | 'v876' | 'v877' | 'v878' | 'v879' | 'v880' | 'v881' | 'v882' | 'v883' | 'v884' | 'v885' | 'v886' | 'v887' | 'v888' | 'v889' | 'v890' | 'v891' | 'v892' | 'v893' | 'v894' | 'v895' | 'v896' | 'v897' | 'v898' | 'v899' | 'v900' | 'v901' | 'v902' | 'v903' | 'v904' | 'v905' | 'v906' | 'v907' | 'v908' | 'v909' | 'v910' | 'v911' | 'v912' | 'v913' | 'v914' | 'v915' | 'v916' | 'v917' | 'v918' | 'v919' | 'v920' | 'v921' | 'v922' | 'v923' | 'v924' | 'v925' | 'v926' | 'v927' | 'v928' | 'v929' | 'v930' | 'v931' | 'v932' | 'v933' | 'v934' | 'v935' | 'v936' | 'v937' | 'v938' | 'v939' | 'v940' | 'v941' | 'v942' | 'v943' | 'v944' | 'v945' | 'v946' | 'v947' | 'v948' | 'v949' | 'v950' | 'v951' | 'v952' | 'v953' | 'v954' | 'v955' | 'v956' | 'v957' | 'v958' | 'v959' | 'v960' | 'v961' | 'v962' | 'v963' | 'v964' | 'v965' | 'v966' | 'v967' | 'v968' | 'v969' | 'v970' | 'v971' | 'v972' | 'v973' | 'v974' | 'v975' | 'v976' | 'v977' | 'v978' | 'v979' | 'v980' | 'v981' | 'v982' | 'v983' | 'v984' | 'v985' | 'v986' | 'v987' | 'v988' | 'v989' | 'v990' | 'v991' | 'v992' | 'v993' | 'v994' | 'v995' | 'v996' | 'v997' | 'v998' | 'v999';",
        ),
        (
            "normalisation: an optional instantiated hook (finding 2)",
            "declare function useLookup<T>(): T;\nexport const Label = (props: Props) => { (useLookup<string>)?.(); return props.label; };",
        ),
        (
            "normalisation: an optional instantiated member hook",
            "declare const hooks: { useLookup<T>(): T };\nexport const Label = (props: Props) => { (hooks.useLookup<string>)?.(); return props.label; };",
        ),
        (
            "normalisation: an optional instantiated forwardRef",
            "import { forwardRef } from 'react';\nexport const Button = (forwardRef<HTMLButtonElement, Props>)?.((props, ref) => <div />);",
        ),
        (
            "normalisation: an optional instantiated member forwardRef",
            "import * as React from 'react';\nexport const Button = (React.forwardRef<HTMLButtonElement, Props>)?.((props, ref) => <div />);",
        ),
        (
            "normalisation: an optional call with its own type arguments",
            "declare function useLookup<T>(): T;\nexport const Label = (props: Props) => { useLookup?.<string>(); return props.label; };",
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
