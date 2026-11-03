---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

`caliper-docgen watch` no longer copies the whole output on every file change before it rewrites `--out`; each change is faster by the cost of that copy, which grows with the size of the project.
