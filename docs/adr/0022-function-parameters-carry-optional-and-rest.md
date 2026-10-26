# 0022. Function parameters carry `optional` and `rest`, and the canonical JSON moves to version 3

**Status:** Accepted **Date:** 2026-10-26

## Context

A function type lost two things about its parameters. OXC keeps `...rest` apart from the other parameters, and the extractor read only the others, so `(...args: any[]) => void` printed as `() => void`. It also dropped the `?`, so `(a?: string) => void` printed as `(a: string) => void`. A caller reading the signature, Storybook or an agent, then believes an argument is required or that none are accepted. The type-loss measurement on current `main` (PR #230) counted rest parameters lost on Mantine 380, antd 140 (102 distinct props), Chakra 15,182 (repeats of the same style props), shadcn 12 and Spectrum 12.

## Decision

A parameter is required, optional (`a?: T`) or the rest parameter (`...a: T[]`). The extractor records the kind per parameter next to its name (`CollectedType::Function::param_kinds`, cache schema 25), and the rest parameter is appended last with its array type as written. `FunctionParam` gains `optional` and `rest`, written to the canonical JSON only when true. `raw_string` prints `a?: T` and `...a: T[]`, which is what the RDT format shows. TOON prints `T?` and `...T`. `structural_hash` includes both flags, so two signatures that differ only by a flag are not interned together.

`SCHEMA_VERSION` moves from 2 to 3. ADR 0017 says not to bump for an addition a reader can ignore. This one is not ignorable: a version 2 reader would show `...args: any[]` as a required parameter named `args`, a wrong signature, and would never have seen a rest parameter before. The Vite plugin and the Storybook addon read version 3 only.

## Not done here

- **`readonly`:** the extractor peels `readonly` from `readonly string[]` on purpose (`extractor/mod.rs`), and the loss table counts it on 540 distinct Chakra types, 101 shadcn, 26 antd and 33 Spectrum. Carrying it needs a flag on `PropType::Array` and `Tuple` and on their collected forms, in the union normalizer, the evaluator's assignability and every printer. A control for an array is the same either way, so the gain is signature fidelity only.
- **`RefObject` shown as `Ref`:** `resolver/react.rs` maps `Ref`, `RefObject`, `ForwardedRef`, `MutableRefObject`, `RefCallback` and `LegacyRef` to one `PropType::Ref { element }`. Telling them apart needs a kind on that variant. Sixteen distinct shadcn types print `Ref<T>` for `RefObject<T>`.

Either would be another schema bump, so they should land together.

## Consequences

- Every installed reader moves with the writer. A mismatch fails at the first read with the message from ADR 0017.
- An old parse cache is discarded once (schema 24 to 25).
- The semantic backend (TS 7.1) does not set the new fields yet: `crates/semantic/src/convert.rs` builds each parameter from its name and type, so its signatures come out with every parameter required.
- Output grows by a handful of bytes per optional or rest parameter, and by the rest parameters that were missing entirely.
