---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A component its module exports under one other name is now reported under that name: `export { _Card as Card }` is `Card`, where it was `_Card` (and `export { Wrapped as DialogTrigger }` is `DialogTrigger`, where it was `Wrapped`). A component with a `displayName`, one exported under its own name or under several names, one reached through a parent, and one whose export name another component of the file already has keep the name they had. The module's own binding stays what `typeof _Card` finds, inside the file and through the export from another module, and the canonical JSON's `exports` is unchanged. `ComponentMapping` gains `local_name`, so the declaration cache is rebuilt.

A watched session no longer extracts a changed file that `.gitignore` excludes. Discovery and the watcher's per-path check now share one walk configuration, so they agree on gitignored files (and on hidden ones, which both keep).
