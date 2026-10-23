---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **A union lists each member once:** a union built from aliases that resolve to the same type prints that type once, as TypeScript does. React Spectrum's collection `children` printed `ReactNode | null | ReactNode | null | …` and now prints `ReactNode | null | (ReactNode | null)[] | …`; `T[] | readonly T[]` prints `T[]`.
- **JSON unions are flat:** in `--json` output a union member that is an alias of a union now contributes that union's members in its place instead of a nested `{"kind":"union"}`. Printed type text is unchanged by this.
