---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

The structural answer (`--no-checker`, and what the checker lane keeps from it) now reads more prop types the way the checker does:

- `Partial<X>`, `Required<X>`, `Pick<X, K>` and `Omit<X, K>` written as a prop's type, over an object literal, a non-generic interface (with the interfaces it extends) or an alias of one, with literal keys, become the object TypeScript computes when every member's type evaluates (MUI's `classes?: Partial<AccordionClasses>` and `slots`). Otherwise the application stays as before.
- `Omit<T[], keyof any[]>` (Chakra's `css` prop) is `{}`.
- `NonNullable<X>` over a known primitive, literal, object, function or array drops `null` and `undefined` (MUI's `tabIndex`).
- `React.ReactElement` and `JSX.Element` stay named `ReactElement` / `JSX.Element` instead of widening to `reactNode`.
- A union holding `any` or `unknown` is that type, as TypeScript reduces it (`T | null` with `T` instantiated as `unknown`).
