---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Recover props shared by both branches of a generic factory's unresolved conditional result, including Mantine's `component` and `renderRoot` props. Preserve payload props when the result still depends on the factory's type parameter.
