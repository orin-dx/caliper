---
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
cargo/caliper-docgen-napi: minor
cargo/caliper-docgen-core: minor
---

When the checker misses `semanticWaitMs` and a story shows caliper-docgen's structural props, the Storybook addon now replaces them with the checker's answer when it arrives, in the story any open browser shows and without a reload: the dev server has Storybook's docgen service ask again, which updates the Controls panel and docs that read it. One answer lifecycle decides when (once per answer, only for a component some browser shows, again once when a browser comes back to one it left, at the newest revision after an edit), and the answer shown is the one a fresh checker session gives. `prefetchStories` now also resolves, after the opening story and after every story you open, the next `prefetchStories - 1` stories' components in the background, nearest first, behind every story you opened; the default stays `1`.

The napi binding's `sessionComponentAnswers` now says, per component, whether the answer awaits the checker (`awaitsChecker`) and the revision of the files it was made at (`revision`), for a lazy and an eager session alike, and the new `sessionCheckerSettled(sessionId, keys, waitMs)` says when the checker has answered. `caliper-docgen-core` exposes `pipeline::awaits_checker(&ComponentAnswer)`, the one place that decides it. With `lazy: false`, the addon now reads a component's diagnostics from its answer, the checker's merged, instead of from the session index: a structural warning the checker superseded no longer hands the component on, and the checker's own warnings do.

Breaking: the Storybook addon options `warmUp` and `fillAfter` are removed (the background prefetch of `prefetchStories` replaces them; left in `main.ts` they are ignored), and the native binding's `sessionWarmUp(sessionId, count)` is removed in favour of `sessionPrefetch(sessionId, keys)`, which resolves the named components instead of a spread of them. Action: drop `warmUp` and `fillAfter` from the addon options; set `prefetchStories` above `1` to resolve the next stories in the background. A caller of `sessionWarmUp` passes the keys it wants to `sessionPrefetch` (`initializeSessionIndex` lists them).
