---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A prop typed with an alias of `Partial`, `Required`, `Omit` or `Pick` keeps that wrapper. `styles?: StylesRecord<'root' | 'item', string>` over `Partial<Record<N, P>>` used to show `Record<'root' | 'item', string>` with every key required, and `Omit<Base, 'x'>` showed all of `Base`; both now read as they do when the utility is written in the prop, which is how Mantine's `classNames` and `styles` appear.
