---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

An interface declared inside a function body, a block or a `switch` case no longer collides with an interface of the same name elsewhere in the file. Before, a module-level interface of that name replaced it, two local ones in different functions overwrote each other, and a component built by a factory showed the wrong props. Each reference now resolves to the nearest interface of its name, the way TypeScript reads it, and declarations of one name in the same scope still merge.
