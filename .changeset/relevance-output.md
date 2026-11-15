---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

The canonical JSON moves to schema version 8: each component gains `relevance`, where relevance evidence (ADR 0028) puts each listed prop (`row`, `used`, `collapsed` or `asToday`) and each member of a collapsed layer that evidence puts forward. Both lanes decide it with one function from the same inputs; which props are listed or collapsed does not change. `ComponentEntry` gains the public fields `relevance` and `relevance_evidence` (opaque `RelevanceEvidence`), and `GlobalSourceData` a crate-internal field (each file's JSX outside component bodies), so it can no longer be built with a struct expression outside the crate. The Vite plugin, the Storybook addon and the scripts read version 8 only. The parse cache moves to version 41.
