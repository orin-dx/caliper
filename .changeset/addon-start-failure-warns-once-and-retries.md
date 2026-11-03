---
npm/caliper-docgen-storybook: patch
---

A Storybook addon that cannot start, because the native binding failed to load or the session failed to index, now warns once at a level Storybook shows instead of logging at debug level, and starts again on the next request instead of caching the failure until the worker restarts. No action needed.
