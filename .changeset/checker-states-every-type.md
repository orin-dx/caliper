---
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With `--experimental-semantic-types`, the checker's answer now covers every kind of type it states, not just the simple ones: tuples with optional, rest and labeled elements, enum members, `bigint` literals, `unique symbol`, template literals, overloaded and constructor signatures, `readonly` properties, and mapped, conditional and utility types as the checker resolved them. A type that recurses is marked as a reference back to itself, and a type cut at a depth or size limit is marked `truncated` instead of standing in as the checker's text. A type from a dependency (an interface, or an object alias) stays a name; the TypeScript lib's utilities such as `Partial` and `Record` are expanded. A union's members come back in the order they were written. The checker is still off by default.
