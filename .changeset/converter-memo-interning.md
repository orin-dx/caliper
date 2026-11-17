---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

**Extraction with the checker uses less memory on large style systems: each converted type is kept once.** The output is unchanged.

- Why it matters: Chakra's style props convert to the same types thousands of times, and each copy was kept. Peak memory with `--load-dependencies`, median of 3: Chakra 1,690 to 1,366 MiB (-19%), Mantine 734 to 716 MiB, shadcn unchanged.
- Limit: a project whose props rarely share types (shadcn) does not change.
