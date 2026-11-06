---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A project with no `typescript` installed now gets a warning that says so, instead of `Date`, `HTMLElement` and `Promise` silently staying unexpanded, and a dependency name that is missing because a re-export cannot be resolved now names that re-export as the cause. Install `typescript` in the project to clear the warning.
