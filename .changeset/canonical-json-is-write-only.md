---
cargo/caliper-docgen-core: minor
---

The core crate no longer reads its canonical JSON back into Rust types, since nothing in Caliper did (the Vite plugin and the Storybook addon read the JSON in TypeScript). `ExtractionOutput` and `PropType` lose their `Deserialize` impls, and the types they hold lose theirs too: `ComponentEntry`, `ParsedProp`, `InheritedLayer`, `DefaultValue`, `PropParent`, `OpaqueReason`, `FunctionParam`, `ObjectField` and `ExtractionStats`. All of them still implement `Serialize`, and the JSON written is byte-for-byte what it was. A library user who parsed that JSON with these types needs their own types for it.
