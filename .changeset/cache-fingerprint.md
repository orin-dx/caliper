---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **A parse cache written by any other build is discarded.** The cache file now starts with the schema version, the crate version and a fingerprint of the code that produces it; any mismatch rebuilds the cache. Before, extractor changes that kept the cached data's shape served stale `.d.ts` results until someone remembered to bump a version, and two tool versions sharing a cache directory could read each other's data.
- A cache from this build that cannot be decoded is reported with an `Info` diagnostic, then rebuilt.
- **`stats.dtsFilesParsed` and `stats.dtsCacheHits` count every `.d.ts` file the run used,** including TypeScript's own lib files. Before, they counted only `.d.ts` files inside the source directories, so they usually read 0.
- The cache file is now `dts.msgpack`; an old `dts-v1.msgpack` is ignored and can be deleted.
