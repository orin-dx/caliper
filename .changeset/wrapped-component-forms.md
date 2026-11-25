---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

**A component is found when its `forwardRef`, `memo` or base class is wrapped in parentheses, a cast or type arguments.** `export const Button = (React.forwardRef<HTMLButtonElement, Props>)((props, ref) => …)` and `class Button extends (React.Component<Props>) {}` list their props, where they were skipped.

- Why it matters: libraries write these forms to satisfy their own typing, and a skipped component shows no Controls.
- Covered: parentheses, `as`, `satisfies`, `!` and type arguments, around the callee or the base class, in `.ts` and `.tsx`.
- Nothing to adopt: output for components that were already found is unchanged.
- Limit: a cast drops a component only when it says the call returns no component or element. `(memo as (c: unknown) => object)(…)`, and casts whose result is a string, number, `void`, a literal or an array of `unknown`, are not listed. Every other cast, such as `(forwardRef as forwardRefType)(…)` or a callable type returning `React.FC<Props>`, is read as the plain call.
- Limit: only a cast to the callee's own type (`typeof` the callee, a `type Factory = typeof createButton` alias, `any` or `unknown`) keeps the callee's declaration. A generic wrapper cast to another signature gets the props of its render function, not the extra props its own declaration adds, and a factory cast that way (`(createButton as SomeFactory)()`) is not listed.
