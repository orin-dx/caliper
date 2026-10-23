---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

**A wrapper-built component gets the props its factory's result type adds:** `const Button = factory<ButtonFactory>(…)` now also lists the props the type `factory` returns declares beyond the payload, such as Mantine's `component` and `renderRoot`, as react-docgen-typescript does. The factory may be reached through `export *` barrels, and editing it updates the components it builds in watch mode. On Mantine's core package, 360 missing props are now reported.
