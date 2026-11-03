---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A project with no `typescript` installed is told so with a warning, instead of `Date`, `HTMLElement` and `Promise` turning into unexpanded names with a hint to check the package is installed. A dependency name behind a re-export whose module cannot be resolved is reported with that cause rather than as "its package's types do not export it".
