---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: patch
---

The on-demand checker answers in three classes: the story shown first, then the stories next to it, then warm-up and fill work, in arrival order within each. A queued request nobody waits for is dropped before the checker converts it when an edit supersedes the revision it was asked at, or when the client no longer shows its component; a request already being converted finishes. For library users: `Priority` has a new `Neighbour` variant, `SharedPriority` gains `raise_to`, `cancel` and `is_cancelled`, and `WatchSession` gains `components_as_neighbours` and `cancel_components`. A caller that names no priority is answered as before.
