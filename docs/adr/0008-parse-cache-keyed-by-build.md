# 0008. Discard the parse cache whenever the build that wrote it differs

**Status:** Accepted **Date:** 2026-10-21

## Context

Under 0003 the cache is discarded only when someone bumps `CACHE_SCHEMA_VERSION`. Extractor changes that keep `SourceData`'s shape but change its content for the same input (#54, #69) were never bumped, so cached `.d.ts` results stayed stale with no signal. The version also lived only in `manifest.json`, a separate file from the payload, so two tool versions sharing a cache directory could pair one's manifest with the other's data.

## Decision

The payload starts with a header: schema version, crate version, and a fingerprint `build.rs` computes from the crate's sources and, in a workspace build, `Cargo.lock`. Loading decodes the header first and discards the cache unless all three match the running build. The manifest repeats the header for inspection only.

- A cache from another build is discarded silently: that is the expected state after any upgrade.
- A cache from this build that fails to decode is discarded with an `Info` diagnostic.
- `CACHE_SCHEMA_VERSION` still moves when `SourceData`'s shape changes, as 0003 says; the golden test pins that.

## Consequences

- Any code or dependency change invalidates the cache, including in development, with no step to remember.
- A cache never survives an upgrade, even one that changed nothing the cache depends on. The cache saves little time per run, so this costs little.
- Builds from the published crate fingerprint the crate's sources without `Cargo.lock`; the crate version covers the dependency set there.
