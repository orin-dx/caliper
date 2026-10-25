---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **A spread in an `as const` array adds the values of the array it names:** `['default', 'primary', 'danger', ...PresetColors] as const` was read as its three literals, so antd's `Button` and `PickerButton` `color` lacked the preset colors. A spread of something that is not an array in the project leaves its values out and warns. The parse cache schema moves to 23.
- **`import React from "react"` names React's types as `import * as React` does:** antd's spelling left `React.CSSProperties["flex"]` and `React.HTMLAttributeAnchorTarget` unresolved. `Flex`'s `flex`, `align`, `justify`, `gap` and `wrap` are now csstype's unions, and `FloatButton`'s `target` is its four anchor targets.
