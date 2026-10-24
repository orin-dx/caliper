---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

- **Breaking: the canonical JSON writes repeated lists once (`schemaVersion` 2).** A component's `props` is now an array of prop set ids into a `propSets` table (prop name to prop id), and an inheritance layer's `omitted` and a diagnostic's `affects` are ids into a `nameLists` table. Chakra with dependencies: 32.1 MB to 4.3 MB, `JSON.parse` 103 ms to 12 ms, and the Storybook addon's session index 6.9 MB to 1.8 MB. Readers resolve the ids; the Vite plugin and the Storybook addon already do, and `--format rdt` and `--format toon` are unchanged. See ADR 0018.
