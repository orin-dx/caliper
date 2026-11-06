---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A panic while loading a dependency's types is reported as an `InternalPanic` error diagnostic naming the import and the panic text. The panic was contained but its diagnostic was dropped, so the types it covered stayed unexpanded with no explanation.
