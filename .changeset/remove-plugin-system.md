---
cargo/caliper-docgen-core: minor
---

The unused plugin system is removed from `caliper-docgen-core`: `DocgenPlugin`, `PluginRegistry`, the `plugin` module and the `plugins` field of `PipelineOptions` no longer exist. Nothing in the CLI, the Vite plugin or the Storybook addon could register a plugin, so extraction output is unchanged. Code that implemented `DocgenPlugin` or set `PipelineOptions.plugins` needs to drop it.
