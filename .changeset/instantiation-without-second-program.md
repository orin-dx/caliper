---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

**Extraction with the checker uses less memory: generic components no longer load the project a second time.** The output is unchanged.

- Why it matters: the peak of an extract with the checker on was when generic components (`<Box as="a">`) were answered. Peak memory with `--load-dependencies`: Chakra 1,886 to 1,690 MiB (-10%), shadcn 961 to 867 MiB (-10%), Mantine 856 to 740 MiB (-14%), antd 1,058 to 1,032 MiB.
- Limit: a project without generic components does not change.
