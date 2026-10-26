---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Lazy sessions now reuse the project index as dependencies load, making repeated requests faster and reducing memory use. Small sessions no longer load every remaining dependency name.
