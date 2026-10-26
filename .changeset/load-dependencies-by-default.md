---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

Installed package types now load by default, revealing more props and reducing unresolved warnings. Use `--no-load-dependencies` or `loadDependencies: false` to keep the previous behavior. On Chakra, extraction rises from about 50 to 220 ms and from 100 to 330 MiB.
