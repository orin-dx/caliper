---
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

`sessionComponentToon` now throws for a session id that is unknown or closed, as the other per-session queries do, instead of answering `null`. It still answers `null` for a session that has no component under the key.
