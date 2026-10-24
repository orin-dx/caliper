---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **More defaults are the components':**
  - `const { dot = false, overflowCount = 99 } = props` in the body documents those defaults, as a destructured parameter does (antd's `Badge`, `Sider`, `Modal`, `Progress`). A renamed one (`tone: t = 'a'`) is the prop `tone`, not the local `t`, in both places.
  - The same default applies to a field of an inline object type in the props (`props: IconProps & { size?: 'S' | 'M' }`, Spectrum's illustrations).
  - A mapped type keeps the JSDoc tags of the member it maps, so a recipe's `/** @default "md" */` becomes the prop's default (Chakra's `size`, `variant`, `shape`).
