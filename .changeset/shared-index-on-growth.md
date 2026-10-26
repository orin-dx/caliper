---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

- **A lazy session grows its dependency layer without copying the project:** the new resolution context shares the project's index, import map, module resolvers and bare-name tables with the one it replaces, and the layer keeps its declarations once. Fifty serial requests take 59 ms on Spectrum where the main branch took 304 ms, 61 ms on antd (168), 106 ms on shadcn (273) and 347 ms on Chakra (436), the slowest single request falls from 18 to 79 ms to 4 to 10 ms (Chakra 70 ms), and peak memory is 81 to 117 MiB, below the batch run's on every corpus.
- **The whole-demand load starts after 48 growths, not 3:** with growth cheap, a session looking at a few dozen components no longer loads every package name (shadcn: 94 MiB instead of 189), while a project read end to end still loads the rest once.
