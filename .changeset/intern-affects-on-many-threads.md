---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Writing the canonical JSON is faster on a project with many diagnostics: the lists of components each diagnostic names are identified on all threads instead of one. On Chakra the write takes about 25 ms instead of 30 ms; the JSON is byte for byte the same.
