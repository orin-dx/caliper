---
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-storybook: patch
---

The experimental TypeScript checker is now built in: `--experimental-semantic-types` and `semanticTypes: 'on-demand'` use the `tsrs` checker linked into Caliper, so there is nothing to install, and a project's own `typescript` or `tsrs` is no longer looked for. It is still off by default. To run another checker, pass `--tsc-path` (or `tscPath`) a `tsc` of `typescript@7.1` or a `tsrs` binary, as before; a project that relied on its own `typescript@beta` being found must now pass it this way. The CLI binary and the native binding are each about 22 MB larger, and their npm packages carry tsrs's `NOTICE`.
