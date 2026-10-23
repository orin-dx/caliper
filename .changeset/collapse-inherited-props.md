---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

- **Large inherited sources collapse to one layer:** with `--load-dependencies`, a type from a package that contributes more than 30 props is listed as an inherited layer with its prop count and its notable props (`onClick`, `className`, `aria-label`, …) instead of prop by prop, as Storybook shows it. shadcn's components list 7,447 props instead of 12,937. `--collapse-inherited-props` (config, NAPI and Vite `collapseInheritedProps`) chooses `dependencies` (default), `all` (the project's own large sources too: chakra's repository goes from 398,915 props and 27.6 MB to 5,541 and 13.6 MB) or `none`. A component's own props type, a `Pick` from a large source and `keyof` are never collapsed. Without `--load-dependencies` the default changes nothing.
