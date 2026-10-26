---
npm/caliper-docgen-storybook: patch
---

`fillAfter` can finish loading a lazy Storybook session after a chosen number of component requests. It stays off by default because it uses more memory; `warmUp` is also off by default. Failed lookups no longer leave stale story keys.
