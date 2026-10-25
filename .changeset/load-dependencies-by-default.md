---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

- **Dependency declarations are loaded by default:** a prop typed by an installed package resolves instead of staying a named reference with an `UNRESOLVABLE_IMPORT` warning (ADR 0019). Chakra goes from 592 to 479,235 props and from 315 to 0 unresolved warnings, at about 220 ms and 330 MiB instead of 50 ms and 100 MiB; the other four benchmark corpora gain 10 to 40 ms. A project with no packages installed loads nothing.
- **Opt out with `--no-load-dependencies`** (CLI), `loadDependencies: false` (`docgen.config.ts`, the Node binding, the Vite plugin, the Storybook addon). `--load-dependencies` is still accepted and means the default. `PipelineOptions::load_dependencies` now defaults to `true`.
