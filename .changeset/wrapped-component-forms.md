---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

**A component is found when its `forwardRef`, `memo` or base class is wrapped in parentheses, a cast or type arguments.** `export const Button = (React.forwardRef<HTMLButtonElement, Props>)((props, ref) => …)` and `class Button extends (React.Component<Props>) {}` list their props, where they were skipped.

- Why it matters: libraries write these forms to satisfy their own typing, and a skipped component shows no Controls.
- Covered: parentheses, `as`, `satisfies`, `!` and type arguments, around the callee or the base class, in `.ts` and `.tsx`.
- Nothing to adopt: output for components that were already found is unchanged.
- Limit: a generic component given type arguments where it is aliased, wrapped or exported (`const RowList = List<Row>`, `memo(List<Row>)`, `Object.assign(List<Row>, { Item })`, `export default List<Row>`) is still not listed, as before; a skip diagnostic names the binding.
- Limit: a cast drops a component only when it says the call returns `object`, `symbol` or `void`, which React cannot render and no component is: `(memo as (c: unknown) => object)(…)` is not listed. Every other cast is read as the plain call, including one whose result is a string, a number, `any[]`, an object type such as `{ type: string; props: {}; key: string | null }`, a union or a type reference.
  - A base class is constructed, not called, so only its construct signatures count: `extends (React.Component as new (props: P) => object)` is not listed, while a cast that adds an unrelated call signature, such as `{ new <P>(props: P): React.Component<P>; (): void }`, still is.
  - For a member such as `(React as SomeType).forwardRef`, only the type the cast gives that member counts, never call signatures of the namespace itself.
- Limit: only a cast to the callee's own type (`typeof` the callee, a `type Factory = typeof createButton` alias, `any` or `unknown`) keeps the callee's declaration. A generic wrapper cast to another signature gets the props of its render function, or the props its type arguments name (`(ui as SomeType).withContext<PublicProps>(Base)` lists `PublicProps`), not the extra props its own declaration adds. A factory cast that way (`(createButton as SomeFactory)()`) is not listed.
  - A component built by a rendering factory called through a type assertion is reported with a `SKIPPED_CANDIDATE` diagnostic instead of props, unless the assertion says the call returns `object`, `symbol` or `void`.
  - An alias counts only when the file declares its name once at the top level as a type: a factory cast to an alias that is also imported or merged with an interface is not listed. The same name declared again inside a namespace, function, class, block or `catch`, or as a type parameter, hides the alias only for a cast inside that scope.

- Limit: React's and Preact's factories (`createElement`, `cloneElement`, `createRef`, the JSX runtime's `jsx`, Preact's `h`, `createContext`, `memo`, `forwardRef`) are recognised exactly as before. An element or ref builder written through an alias, a destructuring, a `require` or a shadowed name is listed as before.
  - A generic call to an element or ref builder, such as `React.createElement<ButtonProps>(Base, props)`, is still listed as a component when written plainly, as before. Through a cast, such as `(React.createElement as typeof React.createElement)<ButtonProps>(Base, props)`, it is still left out, as before.
