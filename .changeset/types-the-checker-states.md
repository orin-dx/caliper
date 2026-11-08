---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
cargo/caliper-docgen-semantic: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

Prop types now keep more of what TypeScript says about them. Tuples keep their optional and rest elements and their labels: `[string, ...number[]]` used to print as `[string, number[][]]`, and `[head: string, ...tail: number[]]` used to come out partly unreadable. A spread of a known tuple, `[string, ...Tail]`, is flattened as TypeScript does. Bigint literals (`10n`), negative number literals (`-1`), `unique symbol`, enum members (`Size.Small`, offered to Storybook as its value `"small"`) and template literal patterns (`` `${number}px` ``) are represented instead of being marked opaque. An object type with several call signatures, a `new` signature, or a call signature beside properties keeps them all, and a recursive alias such as `type Json = string | Json[]` refers back to itself by name instead of being cut off. A getter is read as a property (`get size(): number` is `readonly size: number`), not as a method.

The JSON output moves to schema version 5. A tuple's elements are now `{ "type", "label"?, "modifier"? }` objects, the type kinds `bigintLiteral`, `uniqueSymbol`, `enumMember`, `templateLiteral`, `callable`, `recursive` and `truncated` are new, and the `templateLiteral` reason of an `opaque` type is gone. Update `caliper-docgen`, the Vite plugin and the Storybook addon together; a mismatched install stops with a message naming both versions.
