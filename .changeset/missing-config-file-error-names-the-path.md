---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

A `--config` path that names no file now fails with `config file <path> does not exist` and a hint to check the path, instead of blaming node. No action needed.
