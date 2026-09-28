# 0002. Hand-write Serialize/Deserialize for PropType instead of deriving it

**Status:** Accepted **Date:** 2026-07-22 (retroactive)

## Context

`PropType` and `CollectedType` are recursive enums — variants nest through `Vec`/`Box`/`Option` back into the same type (`Union`, `Object`, `Array`, ...). This project once derived `Serialize`/`Deserialize` directly on a tagged `PropType` (`#[serde(tag = "kind")]`) and had to raise `#![recursion_limit]` to 2048 to get it to compile — each nesting level wraps the generated serializer in another layer during codegen, and the default limit (128) isn't enough for a real component tree.

## Decision

`PropType` and `CollectedType` don't derive `Serialize`/`Deserialize`. Each has a hand-written impl (`to_json_value`/`from_json_value`) that builds a `serde_json::Value` directly instead of going through serde's derive machinery.

**Correction (2026-08-12):** this ADR originally listed `OpaqueReason` alongside `PropType`/`CollectedType` as hand-written. That was wrong — `OpaqueReason` derives `Serialize`/`Deserialize` normally (`crates/core/src/types/output.rs`) and isn't recursive (every variant holds a `String` or nothing), so it was never subject to the recursion-limit problem this ADR is about. What's manual is `PropType::to_tagged_value`/`from_tagged_value` assembling `OpaqueReason`'s JSON shape by hand as part of `PropType`'s own impl — `OpaqueReason` itself needs no special treatment. Found while building `docs/architecture/types-and-output-contract.md`.

## Consequences

- Adding a new variant means updating the manual match in both directions by hand — the compiler won't remind you the way a derive would. `resolver/mod.rs`'s "no wildcard matches" convention is what catches a missed arm here instead.
- `CollectedObjectField` (a struct nested inside `CollectedType::Object`, not an enum) still derives normally — the recursion problem is specific to the recursive _enums_, not every type that touches them.
- The `#![recursion_limit = "2048"]` attribute that used to sit in `crates/core/src/lib.rs`, left over from the old derive-based design, has since been deleted — confirmed absent as of this correction. The full workspace builds and every test passes without it.

## Alternatives considered

`serde(remote)`, or just keeping the recursion-limit bump — both keep the derive, so you inherit the compile-time cost and a cryptic "recursion limit exceeded" the moment someone adds one more level of nesting. Manual impls trade a bit of hand-maintenance for a predictable compile time.
