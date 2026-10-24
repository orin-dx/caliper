---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Defaults destructured from `props` in the body are the component's:** `const { dot = false, overflowCount = 99 } = props` documents `dot` and `overflowCount` with those defaults, as a destructured parameter does (antd's `Badge`, `Sider`, `Modal`, `Progress`). A renamed one (`tone: t = 'a'`) is the prop `tone`, not the local `t`, in both places.
