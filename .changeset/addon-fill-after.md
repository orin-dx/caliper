---
npm/caliper-docgen-storybook: patch
---

- **`fillAfter` resolves the rest of a lazy session in one background pass:** once that many distinct components have been asked for, the remaining components resolve together. It lowers the median story latency of a project read end to end (Spectrum's 50 stories: 19 ms to 5 ms) but not the total time, and raises peak memory by 170 to 420 MB, so it is off (`0`) by default.
- **The background warm-up is off by default (`warmUp: false`):** measured against the same stories it did not make the first story faster, and it held 30 MB more.
- **A listed key with nothing to show is forgotten:** a re-export of a package component that has no props no longer stays in the session, so a failed lookup names the components its file really has.
- The addon README documents `lazy`, `fillAfter` and `warmUp`.
