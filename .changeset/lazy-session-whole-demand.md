---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Long lazy sessions now load remaining dependency types once in the background, making repeated component requests faster. A session that reads the full project may use more memory.
