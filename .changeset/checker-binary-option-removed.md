---
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-storybook: minor
---

The TypeScript checker built into Caliper is now the only one it runs. The option that ran a checker binary as a separate process instead is gone, and with it the version check of that binary. Output with the built-in checker is unchanged.

Breaking: `--tsc-path`, the config's `tscPath`, and the `tscPath` option of the native binding and the Storybook addon are removed. The CLI and the config refuse them as unknown, and the addon refuses `tscPath` by name, since left unread it would quietly run a different checker than the one it named. For library users, `caliper-docgen-semantic` loses `SemanticRequest::tsc_path`, `expected_versions` and `extra_env`, `OnDemandConfig::tsc_path`, `expected_versions` and `extra_env`, `OnDemand::process_id`, `OnDemand::recycles` with `Recycles` (the built-in checker is never replaced), and `SUPPORTED_VERSIONS`, and no longer depends on `caliper-docgen-tsc-client`, which is not released any more; `caliper-docgen-core` loses `PipelineOptions::semantic_tsc_path`.

Action: drop `--tsc-path` and `tscPath`. The built-in checker answers without them; nothing needs to be installed.
