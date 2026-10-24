---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

- **The Storybook addon resolves a component when a story asks for it:** the session indexes where components live from parsing alone and resolves one on request (`lazy`, default `true`; `lazy: false` restores resolving every component at start). Chakra with dependencies answers its first story in 55–121 ms where the eager session took 357–425 ms, and peaks at 419 MB where the eager one peaked at 658 MB. Answers are identical to the eager session's.
- **A background warm-up loads what stories share:** after a lazy session starts, it resolves a few components so dependency declarations are loaded before the first story asks (`warmUp: false` turns it off).
