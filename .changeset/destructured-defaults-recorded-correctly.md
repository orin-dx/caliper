---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A default written in a destructured parameter is now recorded under the prop it belongs to, even when the prop is renamed as in `{ 'aria-label': label = 'x' }`, and defaults like `-1` or `!0` are no longer marked as computed. No action needed.
