---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

Components styled by a call that takes a tag name and a config object, like Chakra's `const Center = chakra("div", { … })` with a `CenterProps` type beside it, are now found with their props. Components report a new `taggedFactory` detection rule. No action needed.
