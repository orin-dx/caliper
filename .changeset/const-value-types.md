---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Props typed from a constant now carry the value TypeScript gives them: `typeof SHOW_PARENT` over `const SHOW_PARENT = 'SHOW_PARENT'` is `"SHOW_PARENT"`, `keyof typeof Icons` over an object literal is the union of its keys, a union of `true` and `false` is `boolean`, and a component exported as `Masonry as (<T>(props: PropsWithChildren<P<T>>) => Element)` has the props of that function type, `children` included.
