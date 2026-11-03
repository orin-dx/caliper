---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A TypeScript checker that stops without answering a request is reported as a warning on the component, as a slow one already was, and a panic while loading a type a dependency declaration refers to is reported as an `InternalPanic` diagnostic instead of ending the load. Before, the first left the structural props with no explanation and the second was not contained.
