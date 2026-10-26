---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Follow imported default component members through dependency barrels and infer props from `React.ComponentType<infer P>` when the component value is known. This restores five props on antd's `InternalPreviewGroup`.
