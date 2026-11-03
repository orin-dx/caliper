---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

A prop typed with a TypeScript enum now shows the union of the enum's values, such as `'primary' | 'secondary'`, instead of an unresolved name with a warning that made `check --strict` fail. No action needed.
