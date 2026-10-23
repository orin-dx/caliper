---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: patch
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: patch
---

- **Watch mode follows configuration changes:** editing a tsconfig, jsconfig, `package.json` or lockfile in or above a source directory (installing a package, changing `paths`) now re-extracts everything and reports every component that changed, instead of needing a restart. `caliper-docgen watch` and the Vite plugin (dev server and `vite build --watch`) watch those files.
- **New API:** `WatchSession::is_resolution_config` and `WatchSession::resolution_config_files` in core, and `isResolutionConfig(sessionId, file)` and `resolutionConfigFiles(sessionId)` in the NAPI binding. `update_file` / `extractFileIncremental` on such a file returns the full delta.
