---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

A prop typed with a TypeScript enum whose members are all literal now shows the union of the enum's values, such as `'primary' | 'secondary'`, instead of an unresolved name with a warning that made `check --strict` fail. Declarations that merge into one enum contribute all their values. An enum with a value that is not a literal, a boolean or an expression, still shows its name with the warning, as does a `declare` enum whose members have no initializer, since its values are unknown. A conditional type that tests a value against an enum, such as `"primary" extends Tone ? number : string`, stays unevaluated, as a string literal is not assignable to a string enum.
