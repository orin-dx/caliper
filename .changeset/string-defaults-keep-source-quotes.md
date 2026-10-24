---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

**String defaults keep their source quotes:** `({ size = 'md' })` gives the default `'md'`, not `"md"`, in every output format, as react-docgen-typescript and Storybook's React docgen print it. A double-quoted or template-literal default keeps its own quotes too.
