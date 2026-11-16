---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen: patch
---

**With the checker on, extraction is faster and uses less memory on components with collapsed layers.** The checker no longer converts the types of props that a collapsed inheritance layer holds, since the layer lists only their names and where they are declared.

- Why it matters: a layer can hold hundreds of props whose types nobody reads.
- Output is unchanged.
- For library users: `SemanticTarget` gains a `layering` field (`None` converts every prop, as before), and `targets_collapsing` builds targets that carry it.
