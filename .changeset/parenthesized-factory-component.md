---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

Components built by a factory call whose render function is wrapped in extra parentheses or a type assertion, like Mantine's `RadioGroup`, `CheckboxGroup` and `SwitchGroup`, are now found, so their props show up in Storybook. No action needed.
