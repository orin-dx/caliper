---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
---

Import and `.d.ts` resolution now run through the TypeScript resolver the checker already links (`tsrs_module`, with `tsrs_tsoptions` reading the tsconfig) instead of a second resolver of our own. The answers are unchanged on the pinned fixtures. A package's declarations, its `@types` package, `exports` conditions, `paths` and `customConditions` are TypeScript's own algorithm, so the structural answer and the checker's cannot disagree about which file a name comes from.

For library users: `ResolutionContext::oxc_resolver` is gone, replaced by a crate-private resolver that is freed with the context.
