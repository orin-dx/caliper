---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With the experimental TypeScript checker on, a prop whose type is a name that does not resolve (an import that cannot be found, a name declared nowhere) is no longer reported as `any`. It is `opaque` and carries `degradedBy: ["unresolvableImport"]`, so a reader can tell it from a prop written as `any`.
