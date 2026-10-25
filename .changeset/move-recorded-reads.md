---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

- **Resolution moves a branch's recorded reads into its parent instead of cloning them:** Chakra with dependencies resolves about 21% faster (343 → 271 ms wall) with the same peak memory and byte-identical output. Spectrum and the smaller projects are unchanged.
