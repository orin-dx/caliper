---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

Real `extract` output validates against the JSON Schema that `caliper-docgen schema` prints. The `detectedBy` list in the schema and in the TypeScript `DetectionRule` type left out `importedMemberAlias`, `memoExoticDecl`, `taggedFactory` and `factoryResult`, so output from components found by those rules failed validation. The schema now lists every Rust variant, and tests fail when the Rust enum, the schema or the TypeScript type differ. Rust code can read the full list from `DetectionRule::ALL`.
