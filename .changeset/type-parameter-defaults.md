---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **An omitted type argument takes its parameter's default:** `interface Props extends DOMAttributes` with `DOMAttributes<T = FocusableElement>` binds `T` to `FocusableElement`, as TypeScript does. Event handlers inherited from a library no longer print an unbound `MouseEvent<T>` (React Spectrum: 1,208 props), and `type Pair<A, B = A[]>` used as `Pair<string>` is `{ first: string; second: string[] }`.
- **The parse cache schema moves to 20** (type parameter defaults are stored with each file's declarations), so existing caches are rebuilt once.
