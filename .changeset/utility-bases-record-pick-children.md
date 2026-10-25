---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

- **A props type built from a utility type lists its props:** `extends Partial<Record<Breakpoint, …>>` gives one prop per literal (antd's `Col` gains `xs` to `xxxl`, and `ContextIsolator` its `form` and `space`), where `Record` was a silent no-op. A key union written as an alias of literals, or as `(typeof ARRAY)[number]` over an `as const` array in another file, is followed. A `string`, `number` or `symbol` key is an index signature and names no prop.
- **`Pick` of one of React's interfaces reads the picked members from React's declaration** whatever `htmlAttributes` is set to: antd's `Progress` gains `aria-label` and `aria-labelledby`, and `CheckableTagGroup` its `className`, `id`, `role` and `style`. A key the declaration lacks is reported. A member declared by a parent interface (`onClick` on `HTMLAttributes`, from `DOMAttributes`) is found, with the element type bound through each hop.
- **`React.PropsWithChildren` with no argument is `{ children?: ReactNode }`:** `React.FC<React.PropsWithChildren>` lists `children` (antd's `UniqueProvider`, Spectrum's `CalendarGrid`).
