---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With `--experimental-semantic-types` or `semanticTypes: 'on-demand'`, a prop whose type the TypeScript checker could not read (a name it did not resolve, a conditional type it left open) no longer has its type replaced by the checker's placeholder when Caliper read that type in full itself: the prop keeps Caliper's type, its `degradedBy` names what the checker could not read, and one info diagnostic counts those props. A type the checker only cut at a size or depth limit is still the checker's. The checker is still off by default. For library users: `merge_semantic_props` returns `MergedProps` (`kept`, and `unread` for these props) instead of a count, and `apply_semantic_props` returns how many props kept their type this way.
