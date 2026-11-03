---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

A `docgen.config.ts` in a package without `"type": "module"` now loads instead of failing with "unknown field `default`". No action needed.
