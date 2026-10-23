---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

- **A library's own declaration wins over a built-in shortcut:** with `--load-dependencies`, a component typed with `HTMLChakraProps<"div">` or Chakra's `ButtonProps` gets the library's real props (139 and 146 of react-docgen-typescript's, against 1 and 0 before), and Mantine's `StylesApiProps` its 8. Shortcuts remain only as fallbacks: when the package isn't loaded, or when its declaration needs a type checker (cva's `VariantProps`).
- **Fewer library shortcuts:** `SxProps`, `SystemStyleObject`, `SystemCssProperties`, `OverridableStringUnion`, `ThemingProps` and `StylesApiProps` are no longer special-cased. Without their package loaded they stay named references with an `UNRESOLVABLE_IMPORT` warning, like any other uninstalled type; `sxProps` is no longer emitted as a type kind.
- **Generic call props:** a component built by a generic factory takes its props from the factory's result type when that resolves, and an `export *` barrel reaches any exported value. A prop two members declare intersects, and `any` absorbs the intersection.
