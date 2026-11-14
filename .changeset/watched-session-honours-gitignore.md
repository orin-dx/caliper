---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A watched session no longer extracts a changed file that a `.gitignore` excludes. Discovery and the watcher's per-path check now build their walk from one function, so they agree on gitignored files (and on hidden ones, which both keep); before, the walk skipped the file and the watcher took it for a source file.
