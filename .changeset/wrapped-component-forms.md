---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

**A component is found when its `forwardRef` or `memo` is wrapped in parentheses, a cast or type arguments, or its base class in parentheses or type arguments.** `export const Button = (React.forwardRef<HTMLButtonElement, Props>)((props, ref) => …)` and `class Button extends (React.Component<Props>) {}` list their props, where they were skipped.

- Why it matters: libraries write these forms to satisfy their own typing, and a skipped component shows no Controls.
- Covered: parentheses, `as`, `satisfies`, `!` and type arguments around the callee, in `.ts` and `.tsx`. Around the base class, parentheses and type arguments.
- Nothing to adopt: output for components that were already found is unchanged.
- Limit: a generic component given type arguments where it is aliased, wrapped or exported (`const RowList = List<Row>`, `memo(List<Row>)`, `Object.assign(List<Row>, { Item })`, `export default List<Row>`) is still not listed, as before; a skip diagnostic names the binding.
- Limit: a cast drops a component only when it says the call returns `object`, `symbol` or `void`, which React cannot render and no component is: `(memo as (c: unknown) => object)(…)` is not listed. Every other cast is read as the plain call, including one whose result is a string, a number, `any[]`, an object type such as `{ type: string; props: {}; key: string | null }`, a union or a type reference.
  - A base class written through a cast, `satisfies` or `!`, such as `extends (Component as typeof Component)<Props>`, is not read, as before: the class is not listed.
  - For a member such as `(React as SomeType).forwardRef`, only the type the cast gives that member counts, never call signatures of the namespace itself.
- Limit: a custom wrapper or factory called through a cast that replaces its type, such as `(ui as SomeType).withContext<PublicProps>(Base)`, `(withTheme as Signature)<Public>(Base)` or `(createButton as SomeFactory)()`, is read exactly as before. The cast is not taken into account, so the props are what they were without this change.
  - Reading the props such a cast gives, with one model for wrappers, tagged factories and factories, is planned as a follow-up.
- Limit: a render function passed to a generic wrapper, as in `(wrap as Signature)<Props>((props: Props) => …)`, keeps the extra props `wrap`'s own declaration adds only through a cast to the callee's own type: `typeof` the callee, a `type Wrap = typeof wrap` alias, `any` or `unknown`. Through any other cast it gets the props of its render function.
  - An alias counts only when the file declares its name exactly once as a type, anywhere in the file. A second declaration anywhere makes the alias stop counting: an import, an interface, a type parameter, a mapped key or a namespace member, in any scope. That includes a declaration TypeScript would keep apart from the alias, so such a cast gets the props of its render function.
  - Through such a cast, `wrap`'s own declaration counts only when the file declares the name `wrap` exactly once, anywhere. An overload, a parameter, an import or a namespace member of the same name means the render function's props only, as before.

- Limit: a cast-wrapped attachment is not detected. A member attached through a cast or assertion is not listed as a member of its parent. That covers a cast on the member, the target, the source object, the `Object.assign` callee or its receiver: `Root.Item = <any>Item`, `Object.assign(Root, { Item: <any>Item })`, `Object.assign(Impl, <const>{ Item })` and `(Object.assign as typeof Object.assign)(Root, { Item })`. Plainly written attachments are read as before.
  - The component behind such a cast is still found under its own name.

- Limit: through a cast or assertion that origin/main did not read, only React's `forwardRef`, `memo` and `lazy` are recognised, bare or on React's namespace, with the import that binds them. Nothing else written that way counts: a custom wrapper, a factory, `createElement`, Preact's `h`, `createContext`, a hook, a `map`, a `Promise`, or a PascalCase helper. Those calls are read as before, so a component that relies on one is not listed.

- Limit: a name read only because a wrapper was looked through counts only when it cannot mean another declaration in the file. Names written plainly are read as before.
  - A member callee written behind a `<T>x` assertion, such as `(<typeof React.forwardRef>React.forwardRef)<R, Props>(…)`, is `forwardRef` only when its receiver resolves to React's import, wherever the import is written in the file.
  - An identifier alias read only through a `<T>x` assertion in a `.ts` file, such as `const PublicRoot = <typeof Root>Root`, counts only when the file declares `Root` exactly once.
  - A receiver written through a wrapper, as in `(Ui as typeof Ui).Button`, is read only when the file declares `Ui` exactly once.

- Limit: React's and Preact's factories (`createElement`, `cloneElement`, `createRef`, the JSX runtime's `jsx`, Preact's `h`, `createContext`, `memo`, `forwardRef`) are recognised exactly as before. An element or ref builder written through an alias, a destructuring, a `require` or a shadowed name is listed as before.
  - A generic call to an element or ref builder, such as `React.createElement<ButtonProps>(Base, props)`, is still listed as a component when written plainly, as before. Through a cast, such as `(React.createElement as typeof React.createElement)<ButtonProps>(Base, props)`, it is still left out, as before.
