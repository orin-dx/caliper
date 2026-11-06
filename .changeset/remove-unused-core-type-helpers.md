---
cargo/caliper-docgen-core: minor
---

Removed `CollectedType::needs_type_checker` and `CollectedType::as_string_union_keys` from the public API. Nothing in the crate used either method, so there is no replacement; match on the `CollectedType` variants directly if you need the same information.
