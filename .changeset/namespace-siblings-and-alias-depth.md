---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Chakra with `--load-dependencies` resolves instead of giving up:** the share of its components with a warning drops from 92% to 3%. Three causes, all fixed:
  - A type declared inside `namespace X {}` that names a sibling (csstype's `DataType.Color = ColorBase | ...`) now finds it. Its members were lost before, in project sources as well as dependencies.
  - `T extends unknown ? Omit<T, K> : never`, the idiom that distributes over a union, takes its true branch, so a props base written that way is no longer opaque.
  - The nesting limit counts one type expression. An alias body starts again at 0, and the declarations a reference passes through are bounded by the names in progress, so a prop 15 levels down no longer runs out of depth inside four csstype aliases.
- The parse cache schema is unchanged.
