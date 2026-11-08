# caliper-docgen-tsc-client

## 0.2.1

- The experimental TypeScript checker can now run on `tsrs` (`npm i tsrs`) as well as on `typescript@beta`. Caliper finds `node_modules/tsrs/bin/tsrs` when no `typescript` 7.1 is installed, and `--tsc-path` (or `tscPath`) also takes the `@tsrs/<platform>/tsrs` binary, which used to fail with "could not determine tsc's version". Only the `tsrs` build that was checked against Caliper is accepted (`0.23.0-ts7.1.0-beta`); any other `tsrs` release is refused with the usual warning until it has been checked. On Chakra, Caliper's output was identical to the one from `typescript@7.1.0-beta`. The checker is still off by default.

## 0.2.0

- The experimental on-demand TypeScript 7.1 checker now works with any 7.1 release (beta, rc or stable) instead of only `7.1.0-beta`, replaces its `tsc` process when it grows past 768 MiB between requests, and no longer makes closing a session wait for a request in progress. For library users: `PINNED_VERSIONS` is now `SUPPORTED_VERSIONS`, and `expected_versions` entries may be a `major.minor` series such as `"7.1"`.

## 0.1.1

- Publish the TypeScript transport and semantic backend crates before the CLI. The CLI's runtime opt-in backend remains available in its crates.io distribution, and every local crate dependency now has a version requirement that Cargo can publish.

