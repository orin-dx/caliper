---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

- **A cold start parses TypeScript's lib files and `@types/react` alongside the project's files:** they were parsed one after the other once the project was done (10–12 ms with an empty parse cache). Output is identical. Cold wall time with an empty cache: Spectrum with dependencies 107 → 93 ms, shadcn with dependencies 121 → 110 ms, shadcn 58 → 50 ms, antd 61 → 55 ms, Mantine 69 → 61 ms. A warm cache is unchanged.
