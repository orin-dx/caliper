---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

**Storybook Controls no longer list hundreds of inherited props.** A source too large to be a component API becomes one inheritance layer of names, so the Controls panel shows the props you care about.

**Breaking:** the Storybook addon's `collapseInheritedProps` now defaults to `'dependencies'` (it was `'none'`). A source of more than 200 props, or an ambient DOM, ARIA or style-system source of more than 30, is one layer of names instead of one Controls row per prop. Your own props and the notable inherited ones are still listed, but a prop inside a layer gets no Controls row, and a story `args` entry naming one shows as undocumented.

**Action:** set `collapseInheritedProps: 'none'` in the addon options to keep every inherited prop listed.

- Chakra's median props per component fall from 845 to 139, and `Button` from about 1,141 Controls rows to about 146.
- A source in your own code collapses by the same rule, so a design system's style props (Chakra's `SystemProperties` in Chakra's own repository) become a layer.
- A collapsed layer never repeats a prop that is listed, with the checker on or off.
