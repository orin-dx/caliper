---
npm/caliper-docgen-storybook: patch
npm/caliper-docgen-vite-plugin: patch
---

A Storybook addon that cannot start (the native binding failed to load, or the session failed to index) now warns once, at a level Storybook shows, instead of logging at debug level, and starts again on the next request instead of caching the failure for the worker's lifetime. `fallback` and `semanticTypes` in the addon and `logLevel` in the Vite plugin fail at startup when they are not one of their choices; a typo used to log everything or act as `fallback: 'never'`. The Vite plugin reports an update it could not apply through the plugin context (`this.warn` in dev, and a failed rebuild under `vite build --watch`) instead of logging and carrying on with stale docs.
