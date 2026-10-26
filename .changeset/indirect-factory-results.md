---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Recover props added by generic component factories when the factory is called through a local alias or its result is returned through a local const. This restores `component` and `renderRoot` on Mantine's `Box` and table components.
