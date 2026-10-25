---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **The canonical JSON names its version:** `ExtractionOutput`, `IncrementalUpdate` and the session index carry `schemaVersion` (1, the shape with the props table). The Vite plugin, the Storybook addon and `ExtractionOutput`'s `Deserialize` reject a version they don't read, with an error that says what to update. `--format rdt` and `--format toon` are unchanged. See ADR 0017.
