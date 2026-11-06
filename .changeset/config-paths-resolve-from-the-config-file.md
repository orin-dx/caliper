---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

Relative `srcDirs`, `tsconfigPath` and `tscPath` in a config file now resolve against the config file's folder instead of the folder the CLI runs from. If you worked around the old behavior with paths relative to the working directory, make them relative to the config file.
