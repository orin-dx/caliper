---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A session with `semanticTypes: 'on-demand'` no longer drops the warning that a prop's type could not be fully resolved when the checker did not return that prop. The prop keeps its structural, partly unresolved type, and its diagnostic now stays with the component instead of disappearing.
