---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

`export const Button = InternalButton` and `memo(InternalButton)` are components even when `InternalButton` is declared further down the file; the alias used to be dropped, and reported as a skipped candidate, whenever it came first.
