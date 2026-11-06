---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A warm run now reflects an upgraded or newly installed dependency instead of the props it cached before, so `--no-cache` is no longer needed after changing `node_modules`. No action needed.
