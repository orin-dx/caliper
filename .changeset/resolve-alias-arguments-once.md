---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Less memory with `--load-dependencies`:** a generic alias reference no longer resolves its arguments before expanding the alias (the expansion reads them again), and equal props built for different components are one allocation. Chakra's peak memory with dependencies loaded falls from 522 to 388 MiB. Output is unchanged.
