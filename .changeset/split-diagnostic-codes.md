---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Two diagnostic codes that were overloaded now have their own. A type that references itself reports `CIRCULAR_REFERENCE` instead of `MAX_DEPTH_EXCEEDED`, which now means only that nesting was too deep. A missing `@types/react` or TypeScript lib file reports `MISSING_TYPE_LIBRARY` instead of `UNRESOLVABLE_IMPORT`. Filters on the old codes need updating. The on-disk parse cache is rebuilt once (schema 6).
