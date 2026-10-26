---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Instantiate generic component props with `unknown` when a namespace member is declared as `typeof Component`, matching direct `ComponentProps<typeof Component>` resolution.
