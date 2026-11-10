---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With `semanticTypes: 'on-demand'`, the warm TypeScript checker keeps its snapshot between requests and brings it up to date with the files the session says changed (`updateSnapshot`), instead of reading the whole program again for every request. Answers are the same bytes as before. An edit nobody announced (a file the checker read changed, and neither `update_file` nor `ensure_fresh` saw it) is found before the checker answers from the old text: that answer is read again from the disk and carries a `SEMANTIC_BACKEND` warning naming the file. A checker warning that comes with an answer is now shown on every answer it applies to; only "unavailable" is still said once per session.

For library users: `SemanticProvider::changed(SourceChange)` is how a `WatchSession` tells its checker what changed (a default that does nothing suits a provider that reads every file afresh), and `OnDemand::snapshots()` counts the snapshots answers came from, `created` from the disk or `updated` from the one before. The checker is still off by default.
