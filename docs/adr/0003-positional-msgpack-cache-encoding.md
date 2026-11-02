# 0003. Cache the DTS parse cache with positional (not named) MessagePack

**Status:** Accepted **Date:** 2026-07-22 (retroactive)

**Note (2026-11-02):** The forgotten-bump risk below now has two backstops. The cache header carries a source fingerprint (`CacheHeader::fingerprint`, set by `crates/core/build.rs`), so any change to the build discards the cache. `cache/golden.rs` snapshots the encoded `SourceData` bytes for the current `CACHE_SCHEMA_VERSION`, so a shape change without a bump fails a test. Field order is still the wire format and the bump rule stands.

## Context

The DTS parse cache persists `SourceData` across runs so unchanged `.d.ts` files don't get re-parsed. `rmp_serde::to_vec`/`from_slice` — the default, and what this project uses — encode structs _positionally_: as an array of field values in declaration order, not a map of field names. A `_named` variant encodes as a map instead, at a size and speed cost we haven't needed to pay.

## Decision

Use plain `rmp_serde::to_vec`/`from_slice`, and treat `SourceData`'s field order as part of the wire format. A `CACHE_SCHEMA_VERSION` constant (`crates/core/src/cache.rs`) gets bumped whenever a field is added anywhere but the end of the struct, removed, or reordered — any change that would shift decode position for cache entries written before the change. A version mismatch discards the whole cache rather than risk decoding an old entry into the wrong field.

## Consequences

- Smaller, faster cache files than named encoding — the whole point.
- Adding a field to `SourceData` (or anything it contains) requires remembering to bump `CACHE_SCHEMA_VERSION` if it isn't appended at the very end. Easy to forget — `const_arrays` (added mid-struct) is exactly the case this version bump was added to cover.
- A forgotten bump doesn't fail loudly. It decodes as plausible-looking but wrong data — the worst kind of bug. There's no compiler check for this; it's a discipline problem, not a type-safety one.
