---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

- **The Storybook addon answers a story up to 3.5× faster on large projects:** it no longer sweeps every source file from JavaScript on each request. The native session checks the files, in parallel, once per story (Spectrum's 129 real stories: 2.2 s → 0.6 s, 15 ms → 4 ms per story).
- **Session start no longer serializes every prop:** the new `initializeSessionIndex` returns only each component's file, name and the diagnostics. Chakra with dependencies skips a 25 MB JSON round trip.
- **The native binding uses mimalloc:** session start is 15–30% faster, and Chakra with dependencies peaks 9% lower (small projects peak 20–30 MB higher).
