---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **cva's `VariantProps` keeps its variants beside other props with `--load-dependencies`:** `React.ComponentProps<"button"> & VariantProps<typeof buttonVariants>` lost `variant` and `size` once the package was loaded, because the loaded declaration needs inference and the shortcut that covers it only stood in when the type was the whole props type. It now stands in for a member of an intersection too (shadcn: 84 components).
