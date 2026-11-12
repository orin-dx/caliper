---
npm/caliper-docgen-storybook: minor
---

When `storybook dev` starts, the Storybook addon now documents the opening story's component through Storybook's docgen service before a browser asks. That starts the docgen worker, the session and, with `semanticTypes: 'on-demand'`, the checker, and stores the answer where the browser reads it, so the first story no longer pays the cold start: on a Chakra project, `Button`'s 1,137 props from the checker were in the browser before it asked, where they arrived 0.7 to 2.9 s after its request before (median 1.6 s). The new option `prefetchStories` (default `1`) sets how many first stories' components are documented, one after another; `0` turns it off. A story opened while a prefetch runs waits for it.
