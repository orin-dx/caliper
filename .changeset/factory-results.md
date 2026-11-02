---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A component built by calling a function is found when that function returns one: `createIconLoader("lucide")`, whose function returns a rendering function with typed props, and `generateCalendar<Dayjs>(config)`, whose generic arrow returns a local asserted to a component type. On shadcn/ui that adds `IconLucide` and four more icon loaders; on Ant Design, `Calendar`. A call to a function that returns anything else is left out, and no longer produces a skipped-candidate note (9 fewer on Ant Design, 18 on shadcn/ui). The new `factoryResult` detection rule appears in `detectedBy`.
