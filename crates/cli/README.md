# caliper-docgen

Extract React component props from the command line. Built on [OXC](https://oxc.rs), so there is no type-checking pass.

```bash
cargo install caliper-docgen
caliper-docgen extract --src src --format rdt
```

| Command               | Does                                                                 |
| --------------------- | -------------------------------------------------------------------- |
| `extract`             | Writes every component's props to stdout or `--out`                  |
| `watch`               | Re-extracts on change                                                |
| `inspect <Component>` | Shows one component's props as a table                               |
| `check`               | Exits non-zero on errors, for CI (`--strict` also fails on warnings) |
| `schema`              | Prints the JSON Schema of the output                                 |

`--format` is `canonical`, `rdt` (the `react-docgen-typescript` shape) or `toon` (compact, for LLM agents). Settings can also live in `docgen.config.ts`, or in the file passed with `--config`.

Prefer npm? `npm install --save-dev caliper-docgen` installs the same binary.

Extraction is in [`caliper-docgen-core`](https://crates.io/crates/caliper-docgen-core). Docs, benchmarks and migration notes: [orin-dx.com/tools/caliper](https://orin-dx.com/tools/caliper/). Source: [github.com/orin-dx/caliper](https://github.com/orin-dx/caliper).

## License

[FSL-1.1-MIT](LICENSE). Each version becomes MIT two years after its release.
