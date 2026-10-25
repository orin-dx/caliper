---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

- **Turn the on-disk caches off:** `--no-cache`, `cache: false` in `docgen.config.ts`, the `cache: false` option of the native binding, the Vite plugin and the Storybook addon. A run with the cache off reads nothing from and writes nothing to the cache directory (neither the declaration parse cache nor the dependency index) and parses everything; the output is the same.
- **Breaking for Rust embedders:** `PipelineOptions` has a new public field, `use_cache` (default `true`). A struct literal that names every field needs it; one that ends in `..Default::default()` does not.
