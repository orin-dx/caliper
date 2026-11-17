---
cargo/caliper-docgen-napi: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-storybook: patch
---

**Storybook answers the story you open first, then its neighbours.** The addon asks the checker for the stories next to the one shown after that story and before warm-up work.

- Why it matters: work for stories you left does not hold up the one you opened.
- Opening another story drops what was queued for the neighbours of the one you left, unless the checker already started it.
- Editing a file makes the addon ask again for the neighbours of the story shown, since the edit dropped what was queued.
- For binding users: `sessionPrefetch` asks at the neighbour class, and the new `sessionCancel(sessionId, keys)` drops the queued requests for `keys` that nobody waits for.
