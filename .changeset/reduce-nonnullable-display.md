---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Chakra's style props no longer show a leftover `NonNullable<…>` in their types: a `NonNullable` over a union whose remaining members are known not to be nullish (primitives such as `number`, objects, and aliases like csstype's `Property.Border<…>`) now prints as the union without `undefined` and `null`, while a member that can itself be nullish, such as a type parameter or an unresolvable name, keeps the call.
