---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
---

Import and `.d.ts` resolution now run through the TypeScript resolver the checker already links (`tsrs_module`, with `tsrs_tsoptions` reading the tsconfig) instead of a second resolver of our own. The answers are unchanged on the pinned fixtures. A package's declarations, its `@types` package, `exports` conditions, `paths` and `customConditions` are TypeScript's own algorithm, so the structural answer and the checker's cannot disagree about which file a name comes from. The dependency index is now keyed to everything a tsconfig contributes to resolution, including the configs it extends and references, so a change in an extended config no longer reads stale answers.

Behaviour change: `baseUrl` no longer affects resolution, as in TypeScript 7 and in the checker. A bare import such as `lib/index` is no longer found through `baseUrl: "."`, and a `paths` entry is relative to its tsconfig, not to `baseUrl`. Projects that use `baseUrl` with `paths` entries written relative to it must move the base into the entries (`"@l/*": ["lib/*"]`).

For library users: `ResolutionContext::oxc_resolver` is gone, replaced by a crate-private resolver that is freed with the context.
