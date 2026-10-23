---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

- **`--format rdt` gives each component its `filePath`:** the absolute path of its source file, as react-docgen-typescript's `ComponentDoc` has. Consumers that group or link components by file no longer need canonical output for it.
