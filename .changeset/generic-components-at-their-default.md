---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With `--experimental-semantic-types` or `semanticTypes: 'on-demand'`, a generic component whose first type parameter declares a default is read at that default, as `<Poly />` renders it: `function Poly<E extends ElementType = 'div'>(props: PolyProps<E>)` now has a `div`'s props and `as: 'div'`, where it had two props and `as` typed by the open parameter `E`. The TypeScript checker forms the instantiation itself, from a file it holds in memory and Caliper never writes to disk. When it cannot (a default the checker prints by a name only the component's module sees, or a checker build that does not read in-memory files), the component keeps its structural props and the checker diagnostic names the instantiation and the checker's error. A type parameter with no default stays open, as before. The checker is still off by default.
