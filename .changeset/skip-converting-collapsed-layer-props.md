---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen: patch
---

The checker no longer converts the types of props that a collapsed inheritance layer holds. It reads only their names and where they are declared, so the output does not change. `SemanticTarget` has a new `layering` field (`None` converts every prop, as before), and `targets_collapsing` builds targets that carry it.
