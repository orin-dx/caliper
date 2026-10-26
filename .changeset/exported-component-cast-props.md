---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Use the asserted props of a component exported through a local cast alias, and resolve React's `PropsWithoutRef<P>` without dropping `P`'s other fields. This restores collection and link props on Spectrum's `Card`.
