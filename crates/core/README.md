# caliper-docgen-core

The extraction library behind [caliper-docgen](https://github.com/orin-dx/caliper). It reads React components from TypeScript with [OXC](https://oxc.rs) and reports their props, with no TypeScript compiler program and no type-checking pass.

It parses files in parallel, merges symbols across them, and resolves each component's props. When it can't resolve something it returns a `Diagnostic` instead of failing silently. There is no terminal output and no async, so it embeds cleanly.

Most people want a front end instead: the [`caliper-docgen`](https://crates.io/crates/caliper-docgen) CLI, or the Node bindings [`caliper-docgen-napi`](https://www.npmjs.com/package/caliper-docgen-napi).

API docs: [docs.rs/caliper-docgen-core](https://docs.rs/caliper-docgen-core). Project page: [orin-dx.com/tools/caliper](https://orin-dx.com/tools/caliper/). Source and architecture notes: [github.com/orin-dx/caliper](https://github.com/orin-dx/caliper).

## License

[FSL-1.1-MIT](LICENSE). Each version becomes MIT two years after its release.
