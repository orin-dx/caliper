---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **A property of a named type's props resolves when its declaration can't be read:** `RcTooltipProps["visible"]` with `type RcTooltipProps = React.ComponentPropsWithoutRef<typeof Tooltip>` is `Tooltip`'s `visible` (antd: `Tooltip.open` and `defaultOpen` are `boolean`; Chakra: `Flex.align`, `Grid.templateColumns` and 54 more style props; shadcn: `Calendar.buttonVariant` and 144 more). The read takes the props the type would have in an `extends` clause and picks the property out, as TypeScript does for an indexed access. An access that finds nothing is reported as before, and a type that reads itself stays opaque.
- **The oracle agrees:** against TypeScript 7.1's checker, props Caliper contradicts fall to 0.00% on every corpus (Mantine was 0.34%, antd 0.25%).
