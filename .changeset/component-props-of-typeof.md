---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **`ComponentProps<typeof Button>` as a component's props resolves to `Button`'s props:** it used to look for a type named `Button` and find nothing, leaving the component with no props (shadcn's `PaginationNext` and `PaginationPrevious`, and any wrapper written this way). It now finds the component, whether it's declared in the same file, imported, or a member of an imported namespace, as the same reference already did in an `extends` clause. `ComponentPropsWithoutRef` and `ComponentPropsWithRef` work the same way; a project's own `ComponentProps` type is unaffected.
