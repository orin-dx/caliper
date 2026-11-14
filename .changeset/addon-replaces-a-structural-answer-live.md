---
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
cargo/caliper-docgen-napi: minor
---

When the checker misses `semanticWaitMs` and a story shows caliper-docgen's structural props, the Storybook addon now replaces them with the checker's answer when it arrives, in the open story and without a reload: the docgen provider waits for the checker, and the dev server has Storybook's docgen service ask again, which updates the Controls panel and docs that read it. At most once per component and change to the files, only for the component on screen (one you left is asked again once when you come back), and the answer shown is the one a fresh checker session gives. `prefetchStories` now also resolves, after the opening story and after every story you open, the next `prefetchStories - 1` stories' components in the background, nearest first, behind every story you opened; the default stays `1`. The options `warmUp` and `fillAfter` are removed: the prefetch replaces them. The napi binding's `sessionWarmUp` is replaced by `sessionPrefetch(sessionId, keys)`, and `sessionCheckerSettled(sessionId, keys, waitMs)` says when the checker has answered.
