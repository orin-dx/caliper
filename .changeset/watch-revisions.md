---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: patch
---

- **Watch output equals a cold extract.** After any edit, creation, rename or deletion, a watch session's snapshot has the same components, enums and diagnostics as `extract` over the same files. Before, edits to a dependency reached through a barrel or a `../` import were missed, deleted files' components stayed until restart, excluded files such as stories added components, a disambiguated component's edit could land under another component's key, and the first run's diagnostics were dropped from later snapshots.
- **Precise re-resolution.** Each component records the files, project-wide names and module resolutions its props depended on; a change re-resolves exactly the components it can affect.
- **`IncrementalUpdate` gains `revision`, `updatedKeys` and `removedKeys`,** so a consumer can apply an update to a keyed map and drop removed entries.
- **Breaking (Rust API):** `WatchSession`'s fields other than `options` and the `ReverseDeps` type are removed; `WatchSession::rebuild()` is new. A deleted file is now a removal, not an `IoError`.
- Resolve diagnostics are ordered by file and source position.
