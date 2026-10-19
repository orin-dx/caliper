# caliper-docgen-vite-plugin

## 0.2.1

- First release under the Caliper names. The crates, the CLI binary and the npm packages were published as `oxc-react-docgen*` before, and are now `caliper-docgen`, `caliper-docgen-core`, `caliper-docgen-napi`, `caliper-docgen-vite-plugin` and their platform packages. The Vite plugin export is `caliperDocgen`. Nothing else changes.

## 0.2.0

- First public release. React component prop extraction on OXC, with no TypeScript `Program` and no type-checking pass: cold extraction across 21 real component libraries in about 50ms and ~2ms for an incremental single-file update.
  
  - `caliper-docgen` CLI with `--format canonical | rdt | toon`, `docgen.config.ts` loading, watch mode, and a `schema` subcommand that exports the component metadata as a Draft-07 JSON Schema. `--format rdt` emits a `react-docgen-typescript`-compatible shape (see `MIGRATING.md`); `--format toon` is a token-optimized output for LLM agents.
  - Cross-package `.d.ts` resolution with an atomic, bounded on-disk cache, and a plugin system (`DocgenPlugin` / `PluginRegistry`) for AST and component resolution hooks.
  - NAPI binding and a Vite plugin that expose the canonical format.
  - An `lsp` subcommand scaffold with bounded framing and the initialize/shutdown/exit lifecycle. It does not serve prop hovers yet.

