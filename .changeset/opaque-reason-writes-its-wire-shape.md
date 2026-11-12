---
cargo/caliper-docgen-core: minor
---

`OpaqueReason` now serializes as the canonical JSON writes it, `{ "type": "runtimeDependent", "functionName": "cva" }`, where its own `Serialize` gave serde's default `{ "runtimeDependent": { "function_name": "cva" } }`. The extraction output is unchanged, since it already wrote the first form. A library user who serialized an `OpaqueReason` on its own gets the new shape.
