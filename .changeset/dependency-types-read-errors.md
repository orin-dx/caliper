---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A dependency declaration file that exists but cannot be read (bad permissions, invalid UTF-8) is now reported as an `IoError` warning naming the file, instead of being skipped without a word, which left the types it declares missing with no explanation. Concurrent runs also no longer share one temporary file when saving the dependency type index.
