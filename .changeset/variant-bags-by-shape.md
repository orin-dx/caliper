---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

Variant props from recipe-style components, such as the `size` and `tone` options of a `cva` or `tv` call, now show up even when the function is imported under another name or comes from a library caliper has never heard of. Nothing to change on your side.
