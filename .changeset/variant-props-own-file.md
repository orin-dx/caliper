---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **`VariantProps<typeof x>` reads the variants of the `x` its own file declares:** two files that each declare an `itemVariants` (shadcn keeps a copy of every component per registry) no longer share the first one's by path, so a component listing `"xs"` for a size its own `cva` does not have is fixed. The declaration the file imports comes next, then the bare name.
