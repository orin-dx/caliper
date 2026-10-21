---
cargo/caliper-docgen-core: minor
---

`ComponentMapping` now records which extractor rule matched it (`DetectionRule`), and `ComponentMapping::new` takes it as a required argument. The on-disk parse cache is rebuilt once (schema 7).
