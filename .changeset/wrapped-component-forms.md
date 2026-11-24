---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

**A component is found when its `forwardRef`, `memo` or base class is wrapped in parentheses, a cast or type arguments.** `export const Button = (React.forwardRef<HTMLButtonElement, Props>)((props, ref) => …)` and `class Button extends (React.Component<Props>) {}` list their props, where they were skipped.

- Why it matters: libraries write these forms to satisfy their own typing, and a skipped component shows no Controls.
- Covered: parentheses, `as`, `satisfies`, `!` and type arguments, around the callee or the base class, in `.ts` and `.tsx`.
- Nothing to adopt: output for components that were already found is unchanged.
- Limit: a cast that gives the callee a different type is trusted over the callee's name. `(memo as (c: unknown) => object)(…)` is not listed, and a generic wrapper cast to another signature gets the props of its render function, not the extra props the wrapper's own declaration adds. Casts to `typeof` the callee itself, `any` or `unknown` read as the plain call, and a cast to a named type (`(forwardRef as forwardRefType)(…)`) still counts as a call to that callee.
