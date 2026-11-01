---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Chakra's style props no longer show a leftover `NonNullable<…>` in their types: a `NonNullable` over a union whose remaining members are named types, such as csstype's `Property.Border<…> | undefined` inside a fallback list, now prints as the union without `undefined` and `null`, while a member that can itself be nullish keeps the call.
