---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

Components built with `forwardRef` now list `ref` however the call is written: `forwardRef(Inner)` as in Ant Design's `Breadcrumb`, a render function without type arguments as in React Spectrum's `TreeView`, or React Aria's `(forwardRef as forwardRefType)(…)`. No action needed.
