---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A type of your own named `K` used as the value of a `Record` now keeps its shape instead of becoming the key, and a generic parameter now shadows a module-level type of the same name everywhere it is used. No action needed.
