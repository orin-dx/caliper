---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

`Pick` and `Omit` now understand any key expression TypeScript code commonly uses: string literals and unions of them, a key alias (`type Keys = "a" | "b"`), `keyof Other`, `Exclude<…>` and `Extract<…>`, a mix such as `"a" | More`, and keys passed through a generic parameter. Before, an alias key set picked nothing or omitted nothing, silently. When the keys can't be known statically, a warning says so. The base can also be any type now, including an intersection such as `Omit<A & B, "k">`. In `caliper-docgen-core`, `CollectedTypeAlias::Pick` and `::Omit` carry a structured `keys` type instead of key lists. The on-disk parse cache is rebuilt once (schema 12).
