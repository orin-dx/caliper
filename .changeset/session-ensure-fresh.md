---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen-napi: minor
---

- **New API for hosts without a file watcher:** `WatchSession::ensure_fresh` in core and `ensureFresh(sessionId)` in the NAPI binding stat every file the session read (source files, the directories they live in, tsconfigs, `package.json` files and lockfiles), apply what changed since the last call, rebuild when a resolution config changed, and return one `IncrementalUpdate`. The revision is unchanged when nothing changed. A docgen provider in a worker, such as Storybook's, can call it before each read.
