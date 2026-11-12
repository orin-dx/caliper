---
npm/caliper-docgen-storybook: patch
---

The Storybook addon now starts in a real `storybook dev`. Storybook hands an addon's preset its own options together with the addon's, and its own `cache` (its file-system cache) reached the native session as the addon's `cache` option, which the session refused, so every story fell back to Storybook's own docgen with "could not start: Failed to convert napi value into rust type `bool`". The addon now reads `cache` only when it is a boolean you set.
