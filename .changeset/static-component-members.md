---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Resolve `ComponentProps<typeof Package.Member>` when an installed package declares `Member` as a static component on a callable export. Wrappers around cmdk's `Command` members now include their props.
