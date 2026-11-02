# caliper-docgen-tsc-client

## 0.2.0

- The experimental on-demand TypeScript 7.1 checker now works with any 7.1 release (beta, rc or stable) instead of only `7.1.0-beta`, replaces its `tsc` process when it grows past 768 MiB between requests, and no longer makes closing a session wait for a request in progress. For library users: `PINNED_VERSIONS` is now `SUPPORTED_VERSIONS`, and `expected_versions` entries may be a `major.minor` series such as `"7.1"`.

## 0.1.1

- Publish the TypeScript transport and semantic backend crates before the CLI. The CLI's runtime opt-in backend remains available in its crates.io distribution, and every local crate dependency now has a version requirement that Cargo can publish.

