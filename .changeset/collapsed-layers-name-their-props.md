---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

A collapsed inheritance layer now names the props it holds, so a consumer can tell a prop that collapsed into a layer (for example one of the 5,000 `@react-types/shared` props behind a react-aria component) from one that is not there. The canonical JSON layer gains `members`, the id of a list in `nameLists`; it is written only for a collapsed layer and old readers ignore it, so the schema version does not change. The weekly measurement now reports recall with and without the props held in layers.
