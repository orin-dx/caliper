# 0017. Version the canonical JSON, and make readers reject a version they don't read

**Status:** Accepted **Date:** 2026-10-24

## Context

The canonical JSON has changed shape under its readers once already: ADR 0009 replaced inline props with ids, and nothing told a reader of the old shape. Three packages parse it (`caliper-docgen-napi`'s types, the Vite plugin, the Storybook addon), and so will other tools, such as an agent server linking `caliper-docgen-core`. Semver on the packages and changesets does not guard the JSON: a reader and the binding can be installed at different releases, and the mismatch surfaces as `undefined is not iterable` somewhere downstream.

## Decision

Every canonical JSON document carries `schemaVersion`, an integer: `ExtractionOutput` (CLI `--json` and `--format canonical`, NAPI `extractAll`, `initializeSession`), `IncrementalUpdate`, and the session index (`initializeSessionIndex`). `SCHEMA_VERSION` in `crates/core/src/types/output.rs` is the one definition. The shape ADR 0009 describes is version 1.

- **Bump** when a reader of the previous version would misread or lose data: a field removed, renamed or retyped, a field's meaning changed, or references and tables restructured.
- **Don't bump** for additions a reader can ignore: a new field, a new `detectedBy` value, a new diagnostic code. Readers must tolerate those.
- **Readers** accept their own version and reject any other with an error naming both versions. Output with no `schemaVersion` predates it and has version 1's shape.
- **Release:** a bump goes out in a changeset marked breaking for the core crate and every package that reads the JSON, so writer and readers move together.
- `--format rdt`, `--format storybook` and `--format toon` have other tools' shapes and carry no version. The parse cache has its own `CACHE_SCHEMA_VERSION`.

## Consequences

- A mismatched install fails at the first read with a message that says what to update, instead of a type error deep in a consumer.
- Every shape change needs a deliberate bump, a changeset and a reader update. That is the point, and it costs a small coordinated release each time.
- An old reader cannot read new output at all, even the parts that didn't change. We accept that over a reader that half-works.
- `ExtractionOutput`'s `Deserialize` rejects a version other than `SCHEMA_VERSION`. Reading older versions would need a migration per version, which nothing needs today.

## Alternatives considered

- **Semver of the package as the contract:** readers and the binding are separate installs, so the package version does not say which shape a document has.
- **A range or a minor version in the document:** none of the three readers can use a "compatible up to" statement, and the "don't bump" rule already covers compatible additions.
