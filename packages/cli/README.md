# caliper-docgen

React component prop extraction, powered by [OXC](https://oxc.rs). Parses TypeScript natively in Rust, with no TypeScript compiler program and no type-checking pass.

```bash
npm install --save-dev caliper-docgen
npx caliper-docgen extract --src src --format rdt
```

- `--format canonical | rdt | toon`: `rdt` emits a `react-docgen-typescript`-compatible shape, `toon` a token-optimized one for LLM agents.
- `watch` re-extracts on change. `schema` prints the JSON Schema for the output.
- Loads `docgen.config.ts`, or the file passed with `--config <path>`.

This package is a thin wrapper that runs the prebuilt binary for your platform (`caliper-docgen-darwin-arm64`, `-linux-x64-gnu`, and so on, pulled in as optional dependencies). All behavior lives in the Rust CLI.

Docs, benchmarks and migration notes from `react-docgen-typescript`: https://github.com/orin-dx/caliper

License: AGPL-3.0-or-later.
