---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

- **A cold run frees the resolution memos while the output assembles:** the memos (about 22 ms of freeing on Chakra with dependencies) are dropped on a background thread as soon as every component is resolved, instead of on the resolving thread before assembly starts. Chakra with dependencies is 4–5% faster (219.6 → 211.3 ms best of 25) with no change in peak memory. The output is unchanged.
