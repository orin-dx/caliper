# oxc-react-docgen-napi

## 0.2.0

- Lint policy (`unsafe_code`, clippy `all`) moved from per-crate `#![...]` attributes into a single `[workspace.lints]` table in the workspace root, with each crate opting in via `[lints] workspace = true`. `unsafe_code` is now `"deny"` rather than `"forbid"` — no crate currently uses `unsafe`, including the NAPI FFI boundary, but `deny` allows a local `#[allow(unsafe_code)]` override if one is ever genuinely needed there.
  
  Also dropped 11 dependencies that `cargo-machete` confirmed were unused: `indexmap`, `oxc_module_lexer`, `thiserror`, `tokio`, `dirs`, `miette` (core); `anyhow`, `clap_mangen`, `lsp-types`, `tracing-indicatif` (cli); `rustc-hash` (binding). No behavior change — smaller dependency graph and faster builds.
- First public release. React component prop extraction on OXC, with no TypeScript `Program` and no type-checking pass: cold extraction across 21 real component libraries in about 50ms and ~2ms for an incremental single-file update.
  
  - `oxc-react-docgen` CLI with `--format canonical | rdt | toon`, `docgen.config.ts` loading, watch mode, and a `schema` subcommand that exports the component metadata as a Draft-07 JSON Schema. `--format rdt` emits a `react-docgen-typescript`-compatible shape (see `MIGRATING.md`); `--format toon` is a token-optimized output for LLM agents.
  - Cross-package `.d.ts` resolution with an atomic, bounded on-disk cache, and a plugin system (`DocgenPlugin` / `PluginRegistry`) for AST and component resolution hooks.
  - NAPI binding and a Vite plugin that expose the canonical format.
  - An `lsp` subcommand scaffold with bounded framing and the initialize/shutdown/exit lifecycle. It does not serve prop hovers yet.
- An unrecognized `htmlAttributes` value (a typo in `docgen.config.ts` or in the JS options) is now an error naming the value, instead of silently falling back to `curated`, as `reactVersion` already does.
- Joined the `napi-and-binding` group at this version.

