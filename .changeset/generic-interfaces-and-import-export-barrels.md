---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

Two props-resolution gaps are closed. A generic interface now receives its type arguments: `Box<string>` for `interface Box<T> { value: T }` yields `value: string`, including through the interface's own `extends Box<T[]>`; before, `value` stayed the unresolved placeholder `T`. And a barrel file written as `import type { X } from "./x"; export type { X };` is now followed like `export type { X } from "./x"`; before, anything imported through such a barrel failed to resolve. On the Mantine source this raises the share of react-docgen-typescript's own props that Caliper reports from 87% to 92%.
