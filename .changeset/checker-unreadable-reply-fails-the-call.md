---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A reply from the TypeScript checker that is not valid JSON is now reported as a warning on the component right away. Before, the reply was dropped and the request waited out its full timeout, then reported a timeout.
