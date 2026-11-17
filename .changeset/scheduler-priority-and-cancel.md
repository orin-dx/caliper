---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: patch
---

**The on-demand checker answers the story you are looking at first.** It works in three classes: the story shown, then the stories next to it, then warm-up and fill work, in arrival order within each.

- Why it matters: opening a story no longer waits behind work for other stories.
- A queued request nobody waits for is dropped before the checker converts it, when an edit supersedes its revision or the client no longer shows its component. A request already being converted finishes.
- A queued request waits at most a bounded number of picks, so lower classes are not starved.
- For library users: `Priority` has a new `Neighbour` variant, `SharedPriority` gains `raise_to`, `cancel` and `is_cancelled`, and `WatchSession` gains `components_as_neighbours` and `cancel_components`. A caller that names no priority is answered as before.
