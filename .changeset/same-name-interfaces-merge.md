---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Two interfaces with the same name in one file now merge their props, as TypeScript does, instead of the later one replacing the earlier. No action needed.
