---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

The default CLI and NAPI output now keeps a dependency's own API with its types, such as antd's `onClose`, and only DOM and ARIA attribute families and style systems collapse to a list of names once they pass 30 props. The Storybook addon is unaffected because it already lists every inherited prop.
