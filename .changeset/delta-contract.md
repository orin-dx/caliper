---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

- **Applying every watch update in order now reproduces the snapshot exactly.**
  - `updatedKeys` includes components whose key moved because a same-named component elsewhere appeared or disappeared. Before, a client applying updates kept a stale entry under the moved key.
  - **Breaking:** `IncrementalUpdate.diagnostics` now holds only the diagnostics a change added; the new `removedDiagnostics` lists the ones it removed. Before, it re-sent the changed file's current diagnostics, and fixed diagnostics never left a client's list.
- **The Vite plugin applies and announces changes one at a time,** so `caliper-docgen:update` events arrive in revision order across files. `CaliperDocgenUpdate` gains `removedDiagnostics`.
