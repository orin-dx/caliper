---
cargo/caliper-docgen-core: minor
---

`ComponentMapping` and `SourceData` gain a crate-internal field each (what a component's body and the rest of its file write in JSX), so neither can be built with a struct expression outside the crate any more: use `ComponentMapping::new` and `SourceData::default()`. Extraction output is unchanged; the declaration cache schema moves to 39, so an existing cache is rebuilt once.
