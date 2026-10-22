---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: patch
---

- **Each component reports how it was recognized:** `detectedBy` is one of `functionDeclaration`, `forwardRef`, `fcAnnotation`, `hocWrapper`, `classComponent`, `forwardRefExoticDecl`, `anonymousDefaultExport`, `attachedSubComponent`, `typedFunctionExpression` or `wrapperTypeArgument`. The last two are inferred from a function's shape and what it returns; the rest are explicit declaration forms. It is in the JSON output, the JSON schema and the TypeScript types.
- **Breaking (Rust API):** `ComponentEntry` gains a public `detected_by` field, and `DetectionRule` serializes in camelCase. The parse cache schema moves to 14.
