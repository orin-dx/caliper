---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

The default CLI and NAPI output now keeps a dependency's own API with its types, such as antd's `onClose`. DOM and ARIA attribute families and style systems collapse to a list of names once they pass 30 props, and any other dependency source once it passes 200 (Chakra v3's 705-prop style system). The Storybook addon is unaffected because it already lists every inherited prop.
