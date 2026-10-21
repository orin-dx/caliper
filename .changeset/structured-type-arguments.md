---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

Type arguments are now carried as structured types instead of display text. Several props that were silently wrong now resolve correctly:

- `Pick<Base, "T">` inside a generic interface keeps the key `"T"`.
- A type named `Infinity` or `NaN` is that type.
- Function-typed arguments resolve instead of becoming opaque.
- `extends Required<T>` keeps `T`'s props and marks them required.
- `extends ComponentPropsWithoutRef<typeof Button>` yields `Button`'s props instead of HTML attributes for an element called "typeof button".
- Nested object arguments no longer raise a false circular-reference note.

In `caliper-docgen-core`, `ComponentMapping::props_type_args` and `ExtendsRef`'s `type_args` are now `Vec<CollectedType>`. The on-disk parse cache is rebuilt once (schema 11).
