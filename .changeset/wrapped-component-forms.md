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
- Limit: a custom wrapper or factory called through a cast that replaces its type, such as `(ui as SomeType).withContext<PublicProps>(Base)`, `(withTheme as Signature)<Public>(Base)` or `(createButton as SomeFactory)()`, is read exactly as before. The cast is not taken into account, so the props are what they were without this change.
  - Reading the props such a cast gives, with one model for wrappers, tagged factories and factories, is planned as a follow-up.
- Limit: a render function passed to a generic wrapper, as in `(wrap as Signature)<Props>((props: Props) => …)`, keeps the extra props `wrap`'s own declaration adds only through a cast to the callee's own type: `typeof` the callee, a `type Wrap = typeof wrap` alias, `any` or `unknown`. Through any other cast it gets the props of its render function.
  - An alias counts only when the file declares its name exactly once as a type, anywhere in the file. A second declaration anywhere makes the alias stop counting: an import, an interface, a type parameter, a mapped key or a namespace member, in any scope. That includes a declaration TypeScript would keep apart from the alias, so such a cast gets the props of its render function.
  - Through such a cast, `wrap`'s own declaration counts only when the file declares the name `wrap` exactly once, anywhere. An overload, a parameter, an import or a namespace member of the same name means the render function's props only, as before.

- Limit: a name read only because a wrapper was looked through counts only when it cannot mean another declaration in the file. Names written plainly are read as before.
  - A base class written through a wrapper, such as `extends (Component as typeof Component)<Props>`, is a React class component only when the file does not declare `Component`, or imports it once. A local `class Component` there is not React's.
  - An `Object.assign(Root, { Item })` written through a wrapper (around the callee, `Root`, or the object as in `{ … } as const`) attaches its members only at module level. `Root` and each member named by an identifier must be declared exactly once in the file.
  - Such an `Object.assign` stands for its first argument only, the object it returns, and only when the file declares that name once. `(Object.assign as typeof Object.assign)({}, Item)` is not `Item`.
  - It is read only in its minimal shape: a target and one object literal with plain keys, each written once. A third argument, a spread, a computed or a repeated key means it is not read. It is also not read when the file writes `Object.assign` in any form: `Object['assign'] = …`, `Reflect.set`, `Object.defineProperty` or `Object.defineProperties`, or `Object[key] = …` with a key that is not known.
  - An identifier alias read only through a `<T>x` assertion in a `.ts` file, such as `const PublicRoot = <typeof Root>Root`, counts only when the file declares `Root` exactly once. The same holds for a member assigned that way: `Root.Item = <number>Item` attaches `Item` only when it is declared once.
  - A call written through a cast is read as a well-known wrapper or element factory only when its name resolves to React's or Preact's import.
    - A wrapper named by convention (`memo`, `withX`) behind a `<T>x` assertion must be an import.
    - `(codec.createElement as typeof codec.createElement)(…)` is no factory.
    - A `createElement` taken from a destructuring, a `require`, a local function or another module is no factory when called through a cast, as before.
  - A call written through a cast is no evidence that a function renders unless the name is shown to mean what it says. A `map` through a cast counts only on an array literal (`([1].map as …)(…)`): a local function called `map`, or a `map` on a value of unknown type, does not count. A helper declared to return an element counts only when its name means that one declaration: a destructured prop of the same name breaks it.
  - A write to `Object` through a mutator itself written through a cast, such as `(Object.defineProperty as typeof Object.defineProperty)(Object, 'assign', …)`, counts as a write. So does `Object.setPrototypeOf(Object, …)`.
  - A receiver written through a wrapper, as in `(Ui as typeof Ui).Button`, is read only when the file declares `Ui` exactly once.
  - A well-known name read through a wrapper must resolve to the global or import it names. `(Preact as typeof Preact).h(…)` counts as Preact's `h` only when the import is the file's one `Preact`, so a parameter named `Preact` breaks it. `(Object.assign as typeof Object.assign)(…)` counts as the built-in only when the file neither declares `Object` nor assigns `Object.assign`.
  - A global's name (`Object`, `Promise`) imported from another module is not the global: `import Promise from './codec'` makes `(Promise as typeof Promise).resolve(…)` no promise. A React import, aliased or not, behind a `<T>x` assertion in a `.ts` file counts only when the import is the name's one declaration: a namespace function `fr` beside `import { forwardRef as fr }` breaks it.

- Limit: React's and Preact's factories (`createElement`, `cloneElement`, `createRef`, the JSX runtime's `jsx`, Preact's `h`, `createContext`, `memo`, `forwardRef`) are recognised exactly as before. An element or ref builder written through an alias, a destructuring, a `require` or a shadowed name is listed as before.
  - A generic call to an element or ref builder, such as `React.createElement<ButtonProps>(Base, props)`, is still listed as a component when written plainly, as before. Through a cast, such as `(React.createElement as typeof React.createElement)<ButtonProps>(Base, props)`, it is still left out, as before.
