---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With `--experimental-semantic-types` or `semanticTypes: 'on-demand'`, a prop the TypeScript checker answers for takes its `parent`, `declarations` and JSDoc tags from the checker, and the component its `composes`, instead of from Caliper's own pass. `parent` keeps its meaning: the interface that declares the prop, with the file as it is on disk, and none for a prop of a type literal; a prop picked or omitted from an interface (`Pick`, `Omit`) names that interface. `composes` lists the React types the props type is made of (`RefAttributes`, `HTMLAttributes`), no longer the types Caliper could not follow where the checker did. A union's members the checker sorted are put back in the written order in more cases (a union of string values, a type named through a namespace, an `undefined` Caliper dropped). The checker asks for descriptions in one round trip per component, and once per declaration, so a whole run is faster (938 Chakra and MUI wrappers: about 15 s to 9 s). The checker is still off by default. For library users: `SemanticProp` has `tags`, `parent` and `declarations`, `SemanticComponent` has `composes`, and `merge_semantic_props` takes the component's `composes`.
