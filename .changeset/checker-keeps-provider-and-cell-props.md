---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With the checker on, a component that has no inheritance layer now keeps the `@types/react` props the structural pass lists for it. A context `Provider` (`ChakraDatePickerPropsProvider`, `MuiSelectFocusSourceProvider`) lost `value` and `children`, and a `th`/`td` attribute intersection (`MuiTableCell`) lost `className` and `style`, because the whole-project merge left every `@types/react` prop to a layer these components do not have.
