---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-storybook: patch
---

The TypeScript checker answers sooner, with the same output. In Storybook, the warm-up now loads the project's dependencies into the checker before the first story asks, and each answer is converted on the checker's own thread: the first answer for Chakra UI's `Button` takes about 115 ms instead of 165 ms, and asking for it again after an edit about 119 ms instead of 152 ms. The warm-up holds about 10 MiB more of the checker's memory. A whole-project `extract` gives each of its two checker programs a snapshot of its own, so they no longer wait for each other, and converts the checker's answers with about 60% fewer allocations: on 938 Chakra and MUI components it takes 1.38 s instead of 1.63 s, and 1.05 s instead of 1.19 s for the 787 Chakra components alone.
