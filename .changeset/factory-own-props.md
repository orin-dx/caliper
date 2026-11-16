---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

**Components built by a factory call now list the props the call itself gives them.** Chakra's `chakra('div', { variants })` and `createLeafComponent(Node, function Divider(...) {})` no longer lose their own props.

- Why it matters: Spectrum's missing props fall from 2,216 to 359, and Chakra's from 13 to 11.
- Variants: a style recipe's variant keys become props. `variants: { inline: { true: {...} } }` gives `inline?: boolean`, and option names give a union of string literals.
- Render function: the annotated first parameter of the call's last argument is read as the props type, including destructuring defaults and discriminated unions. `function Divider({size}: {size?: Size})` gives `size`.
- Limit: only a render function proven to be the component's is read.
- Limit: only props the component's props type lacks are added, and a `ref` the factory adds through its result type is not.
- An existing declaration cache is rebuilt once.
