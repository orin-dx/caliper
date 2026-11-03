---
npm/caliper-docgen-vite-plugin: patch
---

The Vite plugin now reports an update it could not apply as a warning in dev and as a failed rebuild under `vite build --watch`, instead of logging to the console and carrying on with stale docs. No action needed.
