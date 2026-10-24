---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **A recursive generic alias is a cycle, not a depth overrun:** `type CssVars<V = CssVariable> = ... | CssVars<V>[]` pinned its argument to a file again at every level, so the recursion never repeated a type and ran 20 levels deep to a warning. It now stops at the second level, as information. Mantine: components with a warning drop from 69% to 3% (every component with a `vars` prop).
