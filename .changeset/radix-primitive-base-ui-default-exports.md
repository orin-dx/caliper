---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Radix and Base UI components resolve with `--load-dependencies`:** shadcn's components with a warning drop from 32% to 10%.
  - `typeof Primitive.div` (Radix's `declare const Primitive: Primitives`, a mapped type over `(typeof NODES)[number]` whose values extend `ForwardRefExoticComponent<PrimitivePropsWithRef<E>>`) has the props of that component with `E` bound to `"div"`. A `declare const` records its declared type as the alias `typeof X`, and the loader takes it with the value.
  - Base UI's `{ [K in keyof ComponentPropsWithRef<"div">]: Wrap<…[K]> }` props base is the element's attribute layer, as `ComponentProps<"div">` is; the wrapper on each value is not applied (an information diagnostic says so).
- **A default export that is an import resolves through it:** `import Tooltip from './Tooltip'; export default Tooltip` is a re-export of `Tooltip`'s default, so antd's `ComponentProps<typeof RcTooltip>` finds `@rc-component/tooltip`'s component (antd: 9% → 5% of components with a warning).
