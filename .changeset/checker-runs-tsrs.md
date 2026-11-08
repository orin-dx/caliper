---
cargo/caliper-docgen-tsc-client: patch
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

The experimental TypeScript checker can now run on `tsrs` (`npm i tsrs`) as well as on `typescript@beta`. Caliper finds `node_modules/tsrs/bin/tsrs` when no `typescript` 7.1 is installed, and `--tsc-path` (or `tscPath`) also takes the `@tsrs/<platform>/tsrs` binary, which used to fail with "could not determine tsc's version". Only the `tsrs` build that was checked against Caliper is accepted (`0.23.0-ts7.1.0-beta`); any other `tsrs` release is refused with the usual warning until it has been checked. On Chakra, Caliper's output was identical to the one from `typescript@7.1.0-beta`. The checker is still off by default.
