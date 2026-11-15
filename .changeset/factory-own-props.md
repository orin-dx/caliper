---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

A component built by a factory call gains the props the call's own arguments give it. A style recipe's variant keys become props (`chakra("div", { variants: { inline: { true: {...} } } })` gives `inline?: boolean`; option names give a union of string literals), and the annotated first parameter of a render function given as the call's last argument is read as the component's props type, as for a function component (`createLeafComponent(Node, function Divider({size}: {size?: Size}) {...})` gives `size`). Only props the component's props type lacks are added; a `ref` the factory adds through its result type is not. The declaration cache schema moves to 40, so an existing cache is rebuilt once.
