---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Cold extraction is 19–35% faster** with identical output. On the calibration corpora: react-spectrum 150 → 97 ms, Mantine 180 → 146 ms, shadcn 68 → 51 ms, chakra-ui 33 → 25 ms, Ant Design 44 → 34 ms.
  - Each import specifier is resolved once per directory instead of once per component.
  - Hot lookups key on the path string instead of hashing a path component by component.
  - Discovery walks in parallel and only canonicalizes symlinks.
- An import that resolves to a non-UTF-8 path now reports an `UNRESOLVABLE_IMPORT` diagnostic instead of being dropped silently.
