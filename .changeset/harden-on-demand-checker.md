---
cargo/caliper-docgen-tsc-client: minor
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

The experimental on-demand TypeScript 7.1 checker now works with any 7.1 release (beta, rc or stable) instead of only `7.1.0-beta`, replaces its `tsc` process when it grows past 768 MiB between requests, and no longer makes closing a session wait for a request in progress. For library users: `PINNED_VERSIONS` is now `SUPPORTED_VERSIONS`, and `expected_versions` entries may be a `major.minor` series such as `"7.1"`.
