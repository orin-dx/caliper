---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-storybook: minor
---

With `semanticTypes: 'on-demand'`, a story whose component the TypeScript checker is already answering (for the warm-up, a fill, or another story) now waits for that answer instead of asking the checker again behind it, and the checker answers the stories you open before warm-up and fill work. `sessionWarmUp` no longer waits for the checker: it queues each component behind the stories, and the answers are kept for the stories that follow. Background work still runs: after 4 story answers in a row it gets the next turn. For library users: `WatchSession::components_in_background` asks at background priority without waiting, `SemanticProvider::request_at` takes a `SharedPriority` and a `Delivery` (with a default that calls `request`), and `OnDemand` answers by priority.
