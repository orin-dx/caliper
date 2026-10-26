---
cargo/caliper-docgen-core: minor
---

Rust callers constructing `FunctionResult` directly must now set `returned_call` (`None` for a declared return type).
