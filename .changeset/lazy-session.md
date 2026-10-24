---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

- **A lazy session resolves a component when it is asked for:** `WatchSession::lazy(options)` parses and indexes every file and nothing more; `component(key)`, `component_in(file, name)`, `components(keys)`, `docgen_info(key)` and `component_keys()` answer from there. Each answer is the entry and the diagnostics a batch extraction publishes for that key. With `loadDependencies`, a request loads only the dependency names its component reaches. Chakra with dependencies and full HTML attributes starts in about 50 ms and 110 MB instead of 340 ms and 460 MB.
- **NAPI:** `lazy: true` on `createSession`, `initializeSession` or `extractFileIncremental` makes the session lazy, `sessionDocgenInfo` resolves the requested components, and `sessionComponentKeys` and `sessionStats` are new. Without `lazy`, nothing changes.
