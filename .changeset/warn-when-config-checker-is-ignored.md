---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

`check`, `inspect` and `watch` now say so when the config sets `experimentalSemanticTypes` or `tscPath`: those commands never run the TypeScript checker, and the setting used to be accepted and ignored without a word. The note goes to stderr, and `--quiet` silences it.
