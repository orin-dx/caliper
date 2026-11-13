# caliper-docgen

Extract React component props from the command line. TypeScript's checker (the `tsrs` crates, built in) decides each prop's type and [OXC](https://oxc.rs) reads the syntax; `--no-checker` runs the faster structural resolver alone.

```bash
npm install --save-dev caliper-docgen
npx caliper-docgen extract --src src --format rdt
```

| Command               | Does                                                                 |
| --------------------- | -------------------------------------------------------------------- |
| `extract`             | Writes every component's props to stdout or `--out`                  |
| `watch`               | Re-extracts on change, structural only (no checker)                  |
| `inspect <Component>` | Shows one component's props as a table                               |
| `check`               | Exits non-zero on errors, for CI (`--strict` also fails on warnings) |
| `schema`              | Prints the JSON Schema of the output                                 |

`--format` is `canonical`, `rdt` (the `react-docgen-typescript` shape) or `toon` (compact, for LLM agents). `extract`, `check` and `inspect` wait for the checker: about 6 s for an app of 938 Chakra UI and MUI components, against 0.1 s with `--no-checker`. Settings can also live in `docgen.config.ts`, or in the file passed with `--config`.

This package is a thin wrapper. It runs the prebuilt binary for your platform, installed as an optional dependency (`caliper-docgen-darwin-arm64`, `caliper-docgen-linux-x64-gnu`, and so on). All behavior is in the Rust CLI.

Docs, benchmarks and migration notes: [orin-dx.com/tools/caliper](https://orin-dx.com/tools/caliper/). Source: [github.com/orin-dx/caliper](https://github.com/orin-dx/caliper).

## License

FSL-1.1-MIT. Each version becomes MIT two years after its release.
