---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

The default output of the CLI and the NAPI binding now lists a dependency's own API with its types. Until now any dependency source over 30 props collapsed to a list of names, so a prop such as antd's `visible`, `onClose`, `value` or `onChange` (from `@rc-component` types) or Base UI's `open` and `items` (from `ComboboxRootProps`) arrived as a name with no type. An ambient source collapses past 30 props: DOM and ARIA attribute families (react-aria's `GlobalDOMEvents`, `*HTMLAttributes`) and style systems (`*StyleProps`, `SystemProps`). Any other dependency source collapses only past 200 props (Chakra v3's 705-prop style system). The Storybook addon is unaffected: it already lists every inherited prop (`collapseInheritedProps: 'none'`).
