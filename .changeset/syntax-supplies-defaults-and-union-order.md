---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With `--experimental-semantic-types` or `semanticTypes: 'on-demand'`, a prop the TypeScript checker answers for takes its default and the order of its union's members from the source itself: the default the component's code gives it (a destructured parameter's initializer, `defaultProps`), else its `@default` tag, and the members in the order its declaration writes them. Props only the checker returns now have both too (938 Chakra and MUI wrappers: 834 more defaults, 1,452 unions in written order), and a few unions Caliper's own pass ordered wrong are fixed (`slot?: string | null`). Members that come through an imported type keep Caliper's own order, as before. For library users: `merge_semantic_with` takes a `ResolverFacts`, `MergedProps` has `diagnostics` and is no longer `Copy`, `apply_semantic_props` is gone (`merge_semantic` does what it did), and the merge functions read the source files the answer names.
