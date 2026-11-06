---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Components cast to `VFC` or `VoidFunctionComponent` now keep their props, and Preact components that return a type from `preact/compat` or render with `h()` are now found. No action needed.
