---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

A config whose default export is a function or a promise is now called and awaited instead of being ignored or failing with a parse error, and a default export that is not an object fails with a message saying what it is. No action needed.
