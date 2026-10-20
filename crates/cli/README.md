# caliper-docgen

Command-line React component prop extraction, powered by [OXC](https://oxc.rs).

```bash
cargo install caliper-docgen
caliper-docgen extract --src src --format rdt
```

- `--format canonical | rdt | toon`: `rdt` emits a `react-docgen-typescript`-compatible shape, `toon` a token-optimized one for LLM agents.
- `watch` re-extracts on change. `schema` prints the JSON Schema for the output.
- Loads `docgen.config.ts`, or the file passed with `--config <path>`.

Prefer npm? `npm install --save-dev caliper-docgen` installs the same binary.

Extraction itself is in [`caliper-docgen-core`](https://crates.io/crates/caliper-docgen-core). Docs, benchmarks and migration notes from `react-docgen-typescript`: https://github.com/orin-dx/caliper

License: AGPL-3.0-or-later.
