---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-storybook: minor
---

With `semanticTypes: 'on-demand'`, a story whose component Caliper's own pass lists no props for (such as `ComponentProps<typeof Button>` of a library it cannot read, which is most of Chakra) now waits up to 2 seconds for the TypeScript checker instead of showing no props after 150 ms: Chakra's `Button` now shows the checker's 1,138 props on its first request. Components Caliper lists props for still answer within `semanticWaitMs`, exactly as before. The new option `semanticWaitEmptyMs` (default `2000`) sets the longer wait; a checker slower than it is still kept for the next request. For library users: `WatchSession::with_semantic_wait_empty` sets the same wait.
