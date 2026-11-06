---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A prop written in an inline object type (`props: { ... }`) keeps its JSDoc tags, such as `@deprecated` and `@example`, as a prop in an interface already did. Before, only its description survived. A method written in an inline object type also keeps its description.
