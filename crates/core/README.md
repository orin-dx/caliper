# caliper-docgen-core

Core library for [caliper-docgen](https://github.com/orin-dx/caliper): extracts React component props from TypeScript with [OXC](https://oxc.rs). No TypeScript compiler program and no type-checking pass, so a cold run over 21 real component libraries takes about 50ms.

Pipeline: parse files in parallel, merge the symbol data, resolve each component's props, return the extraction output. Whenever it degrades it emits a `Diagnostic` rather than failing silently.

This crate is pure extraction logic: no terminal output, no async. The command-line tool is [`caliper-docgen`](https://crates.io/crates/caliper-docgen), and the Node bindings are [`caliper-docgen-napi`](https://www.npmjs.com/package/caliper-docgen-napi).

Docs: https://docs.rs/caliper-docgen-core. Architecture notes and known gaps: https://github.com/orin-dx/caliper

License: AGPL-3.0-or-later.
