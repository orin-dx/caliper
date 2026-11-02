---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A watch session applies a file change faster: it no longer spends the update freeing the revision it replaces. On a large project that was about a quarter of each change; the free now happens on another thread.
