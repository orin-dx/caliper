---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Editing a file a component only imports, such as the one that declares its props interface, now makes the TypeScript checker answer again for that component. A session with `semanticTypes: 'on-demand'` used to keep showing the checker's types from before the edit until the component's own file changed.
