---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

A whole-project `extract` with the checker (the default) is about three to four times as fast, with the same output. On an app wrapping the 938 components of Chakra UI and MUI it takes 1.8 s instead of 6.2 s, and 1.4 s instead of 5.7 s for the 787 Chakra components alone; `react-docgen-typescript` takes 7.3 s and 4.1 s on the same files. The checker's answer for a prop that many components share (Chakra's style props) is now converted and merged once instead of once per component, the answers are read on several threads, and the built-in checker answers two programs over the same files at once. Peak memory is about 10% higher (1.1 GiB instead of 0.98 on that app; 1.5 GiB on three copies of it). The Storybook session and a checker run through `--tsc-path` keep asking one program.
