---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

**A component is found when its `forwardRef`, `memo` or base class is wrapped in parentheses, a cast or type arguments.** `export const Button = (React.forwardRef<HTMLButtonElement, Props>)((props, ref) => …)` and `class Button extends (React.Component<Props>) {}` list their props, where they were skipped.

- Why it matters: libraries write these forms to satisfy their own typing, and a skipped component shows no Controls.
- Covered: parentheses, `as`, `satisfies`, `!` and type arguments, around the callee or the base class, in `.ts` and `.tsx`.
- Also covered: a generic component given type arguments where it is wrapped or aliased, such as `memo(List<Row>)`, `Object.assign(List<Row>, { Item })` or `const RowList = List<Row>`, with or without a cast or `!` around it.
  - The type arguments are applied: `memo(List<Row>)` lists `ListProps<Row>`.
  - Limit: where they cannot be applied (an imported component, a type parameter used inside the props type such as `GridProps<T[]>`, a cast beneath the type arguments), the props keep the type parameters and a `GENERIC_ARGUMENT_MISMATCH` warning names the component.
- Nothing to adopt: output for components that were already found is unchanged.
- Limit: a cast drops a component only when it says the call returns `object`, `symbol` or `void`, which React cannot render and no component is: `(memo as (c: unknown) => object)(…)` is not listed. Every other cast is read as the plain call, including one whose result is a string, a number, `any[]`, an object type such as `{ type: string; props: {}; key: string | null }`, a union or a type reference.
- Limit: only a cast to the callee's own type (`typeof` the callee, a `type Factory = typeof createButton` alias, `any` or `unknown`) keeps the callee's declaration. A generic wrapper cast to another signature gets the props of its render function, or the props its type arguments name (`(ui as SomeType).withContext<PublicProps>(Base)` lists `PublicProps`), not the extra props its own declaration adds. A factory cast that way (`(createButton as SomeFactory)()`) is not listed.
  - A component built by a rendering factory called through a type assertion is reported with a `SKIPPED_CANDIDATE` diagnostic instead of props, unless the assertion says the call returns `object`, `symbol` or `void`.
  - An alias counts only when the file declares its name once at the top level as a type: a factory cast to an alias that is also imported or merged with an interface is not listed. The same name declared again inside a namespace, function or class, or as a type parameter, hides the alias only for a cast inside that scope.
