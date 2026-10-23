---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

**Curated mode also gives a prop beside `ComponentProps<"div">` the element attribute's type:** recharts' `onTimeUpdate` next to a div's attributes is now `ReactEventHandler<HTMLDivElement>` intersected with recharts' handler in the default curated mode too, not only in Full mode. Curated mode reads React's declarations for this lookup only (ADR 0013), so nothing else in its output changes.
