---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

Watch mode now agrees with a fresh extraction. The session snapshot lists enums under their public names (it exposed internal `path:name` keys), an import added by an edit is followed by later edits (the dependency graph was only built once), and a wrapper-call guess that resolves to no props is omitted the same way. The unused public `WatchSession::source_cache` field is removed.
