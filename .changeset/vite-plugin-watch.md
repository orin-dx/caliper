---
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-napi: patch
cargo/caliper-docgen-napi: patch
---

- **The virtual module reloads on every change** with the session's current output: renamed or deleted components disappear, disambiguated components keep their own keys, and enums and diagnostics stay current. Before, it was never invalidated after the first load, entries were keyed by display name, and removals were never applied.
- **`caliper-docgen:update` carries `revision`, `updatedKeys` and `removedKeys`.** A change that produced no new revision (an excluded story, for example) sends nothing. A change that could not be applied is logged and sends nothing; before, an empty update was sent.
- **`vite build --watch` stays current.** Every source directory is watched, so a change to a file no bundle module imports still rebuilds, and each change is applied before the rebuild loads the virtual module. Before, rebuilds served the first build's output.
- **A build fails when extraction fails,** instead of shipping an empty virtual module.
- The `CaliperDocgenUpdate` type is exported.
