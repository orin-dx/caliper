---
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
npm/caliper-docgen: minor
---

`extract` keeps the checker's answer in its cache directory, and a rerun over unchanged files reuses it: on an app wrapping the 938 components of Chakra UI and MUI a rerun takes 0.25 s instead of 1.4 s. Before reusing it, the run asks again every question the checker asked of the disk (each file it read or looked for, each directory it listed, the tsconfig and what it extends); any different answer, a new file where it found none included, discards it whole, as does another Caliper build. `--no-cache` neither reads nor writes it, and `check`, `inspect`, a `--tsc-path` checker and the Storybook session never use it.

A whole-project `extract` with the checker is also faster and smaller when it does ask: each property's type is asked for once per run instead of once per component, and fewer files are asked about at once. On the same app it takes about 1.4 s and 990 MiB instead of 1.7 s and 1,080 MiB; on the Chakra components alone about 1.1 s and 720 MiB instead of 1.35 s and 870 MiB. The output is the same.
