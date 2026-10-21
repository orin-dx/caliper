---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Writing the output is faster, with byte-identical JSON.** Mantine's full run drops from 137 to 90 ms; react-spectrum's from 93 to 71 ms. Prop types are now streamed to the serializer instead of being built as an intermediate JSON tree first.
- The JSON key order of prop types is now fixed by the code. Before, it depended on whether a dependency enabled a `serde_json` feature.
