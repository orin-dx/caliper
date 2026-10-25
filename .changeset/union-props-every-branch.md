---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **A component whose props are a union of object types has the props of every branch:** `(A & { multiple?: false }) | (B & { multiple: true })` (antd's `CascaderPanel`, shadcn's `SidebarMenuButton` and `TraceCollapsible`) kept only the first branch's type for each prop. A prop the branches type differently is now the union of those types (`multiple?: false | true`), and one that some branch lacks or marks optional is optional (`href`, `onOpenChange`).
