---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

A relative `cacheDir` in `docgen.config.ts` resolves against the config file's directory, as `srcDirs` and `tsconfigPath` already did. Before, it resolved against the directory the command ran from, so the same config wrote its cache to different places depending on where it was run. The `--cache-dir` flag stays relative to the working directory.
