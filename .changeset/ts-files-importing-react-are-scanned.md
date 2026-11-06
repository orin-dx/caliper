---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Components built with `createElement` or `forwardRef` in a plain `.ts` file are now found when the file imports a value from React, where they were never looked for before. Files with no React import are still skipped. No action needed.
