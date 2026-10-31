---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

Prop types now keep `readonly` on arrays, tuples and object fields, index signatures such as `{ [key: string]: number }`, and the ref type a prop was written with (`RefObject<T>`, `ForwardedRef<T>`, `RefCallback<T>`) instead of printing them as `string[]`, `{}` and `Ref<T>`. The JSON output moves to schema version 4, which adds a `readonly` type kind, a `name` on `ref` types, and `readonly` and `indexKey` on object fields: update `caliper-docgen`, the Vite plugin and the Storybook addon together, and a mismatched install stops with a message naming both versions. A union such as `string[] | readonly string[]` now lists both members.
