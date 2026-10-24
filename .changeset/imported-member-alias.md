---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **A re-exported member of an imported component is documented with `--load-dependencies`:** `export const CheckboxHiddenInput = ArkCheckbox.HiddenInput` has the props of Ark's `HiddenInput` (Chakra: 11 components that react-docgen-typescript reports). Without the package loaded there is nothing to show, so it is left out. The parse cache schema moves to 21.
