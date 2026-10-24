---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **`Exclude`, `Extract` and `NonNullable` come from TypeScript's own definitions:** they were matched by name and evaluated only over literals. Now `type Exclude<T, U> = T extends U ? never : T` from the project's `lib.es5.d.ts` is evaluated the way TypeScript does, so a conditional alias over a bare type parameter distributes over the members of its argument, through aliases, `(typeof LIST)[number]` and `boolean`. A project's own `type Without<T, U> = T extends U ? never : T` works the same. `T & {}` removes `null` and `undefined`. Without TypeScript installed, these print as written, as before.
- **A generic alias with a long expansion prints by name, as TypeScript and react-docgen-typescript do:** `Responsive<DimensionValue>` instead of its 80 literals three times over (React Spectrum: exact agreement with react-docgen-typescript's type text rises from 67.5% to 78.1%).
- **A union drops `never` and a union of nothing is `never`.**
