---
cargo/caliper-docgen-napi: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-storybook: patch
---

The Storybook addon asks the TypeScript checker for the components of the stories next to the one shown after the story shown and before warm-up and fill work. When you open another story, the checker drops what it had queued for the stories next to the one you left, unless it already started; when you edit a file, the addon asks again for the stories next to the one shown, since the edit dropped what was queued. For binding users: `sessionPrefetch` asks at the neighbour class, and the new `sessionCancel(sessionId, keys)` drops the queued requests for `keys` that nobody waits for.
