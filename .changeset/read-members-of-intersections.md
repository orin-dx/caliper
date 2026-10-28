---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Chakra's style props now show their real token types, such as `SpacingToken` and `ColorsToken`, instead of an unresolved `Tokens["spacing"]`. The same fix applies to any type that reads a property from an object combined with an index signature.
