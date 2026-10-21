---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **A project's own type named like a builtin is that type.** A local or imported `ReactNode`, `CSSProperties`, `Ref`, `Partial` or `Required` now resolves as declared. Before, these were matched by spelling first and read as React's or TypeScript's type, or as an empty utility. Mantine's own `CSSProperties`, which extends React's with an index signature, now appears as itself in `style` props.
- **React meanings need React's binding; TypeScript utilities need no binding at all.** `PropsWithChildren` and `PropsWithRef` count as React's.
- **When a builtin name is bound to something that can't be resolved,** such as a package that isn't installed, React's meaning is still used, and an `Info` `UNRESOLVABLE_IMPORT` diagnostic says so. react-spectrum's source-only mirror reports 29 of these, for types from `@react-types/shared`. Before, they were silent.
