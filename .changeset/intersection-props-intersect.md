---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **A prop two intersection members declare has both types:** `A & B` used to keep whichever member declared a prop first, dropping the other declaration. As in TypeScript, the prop now has the intersection of both types (literal unions reduce to their common values, so `("default" | "destructive") & ("default" | "info")` is `"default"`), is required when either member requires it, and lists both declarations. An interface's own declaration still replaces the one it extends.
- **In Full mode, a prop beside `ComponentProps<"div">` also has the element attribute's type:** recharts' `onTimeUpdate` next to a div's attributes is `ReactEventHandler<HTMLDivElement>` intersected with recharts' handler. Curated mode does not read React's declarations, so there the prop keeps the other member's type.
