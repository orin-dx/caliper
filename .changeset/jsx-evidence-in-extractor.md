---
cargo/caliper-docgen-core: minor
---

**Library users: `ComponentMapping` and `SourceData` can no longer be built with a struct expression outside the crate.** Use `ComponentMapping::new` and `SourceData::default()`.

- Extraction output is unchanged.
- An existing declaration cache is rebuilt once.
