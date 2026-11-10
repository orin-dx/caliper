# 0027. Identify a component by its declaration and its exports, not its display name

**Status:** Proposed **Date:** 2026-11-10

## Context

The extractor knows which declaration a component is and which names its module exports it under (`LexedExport::LocalDeclaration { name, declared_as }`, the attached sub-components in `extractor/uses.rs`). It keeps neither: `ComponentMapping` holds `component_name` and `exported: bool` (`types/collected.rs:1739`, set by name at `extractor/mod.rs:620`), and `ComponentEntry` holds `display_name` and `exported` (`types/output.rs:129`). Every reader that needs to know which export a component is works it out again from the display name:

- The checker backend takes the export named like the display name, or like its last `/` or `.` segment, else the module's `default` (`crates/semantic/src/component.rs:54`). With `Menu.Item = Item; export default Menu`, the checker run gives `Item` the props of `Menu` and reports it as resolved by the checker (reproduced with the CLI and tsrs 0.23.0: structural `Item` has `value`, checker `Item` has `label` and `open`).
- The merge and the session pair an answer with its entry by file and display name (`types/semantic.rs:316,360`, `pipeline/semantic_hook.rs:178`). A late answer takes the first component with no pairing at all (`semantic_hook.rs:144`).
- The oracle matches by `normComponent` (`apps/oracle/src/tool-universe.ts:19`), which strips the path, the `.` prefix and every non-alphanumeric character, so `_Card` and `Card`, or `Menu.Item` and `Item`, are one name. A default and a named export of one function were counted twice (review finding on PR 413).
- The Storybook addon compares a story's import name with `displayName` and reads the module's exports with regular expressions (`packages/storybook/src/exports.ts:7,18`).
- The S5 probe prints a generic default as text and reads it in another module, where a module-local `FormData` becomes the DOM global (review finding on PR 410).

Each case was fixed, or is being fixed, in the reader where a review found it. The next reader starts from the display name again. After ADR 0026's S8 the checker's answer is the default output, so a wrong pairing is no longer confined to an opt-in path. The construct suite has one component per file (`Row`), so S7's gate cannot see one.

## Decision

A component's identity is its declaration and its exports. Its display name and output key are for display and for addressing by a host; nothing pairs, dedupes or caches on them.

- The extractor records each component's exports: the names its module exports it under (`default` included), or for an attached sub-component the path through its exported parent (`Menu.Item`). `exported` becomes `!exports.is_empty()`.
- Within a run, the declaration (file and span of the declaring node) is the identity used for dedupe, pairing, probes and caches. Across revisions it stays `ComponentId` (`pipeline/revision.rs:96`), because spans move with edits.
- The checker backend asks for a recorded export by its exact name, follows aliases, and keeps the structural result with a reason when the export is missing or declared in another file. It never falls back to `default` unless `default` is a recorded export.
- A request to the checker carries the output key, and the answer and each fallback carry it back; the merge finds the entry by key.
- The canonical JSON gives each component its `exports`. Readers outside the crate (oracle, addon, scripts) join on file and export name and stop re-deriving names.
- A type argument written into a probe resolves to the declaration the checker read it from: it is either written in the declaring module or checked against that declaration. Printed text is not re-read in another scope unchecked.

## Consequences

- `SourceData` changes, so `CACHE_SCHEMA_VERSION` moves (ADR 0003). The canonical JSON gains a field, so `SCHEMA_VERSION` moves from 6 to 7 with the oracle's `SUPPORTED_SCHEMA_VERSION`, the TypeScript declarations and the `schema` command (ADR 0017). `SemanticTarget`, `SemanticComponent` and `SemanticFallback` are public in `caliper-docgen-core`, so the core crate takes a minor bump.
- The checker's symbol handle names a declaration by node index, kind and path (`crates/semantic/src/convert.rs:252`), not by offset, so the backend checks the file, not the span. A span check would need a request this audit did not find in the protocol.
- The construct suite gains rows with several components in one file: a default and a named export of one function, `export { _X as X }`, an attached sub-component beside a default export, and a default export beside a named one. They gate this change and S7.
- This lands before S7. S7 deletes most of the structural resolver, where the remaining name-capture findings live (ADR 0025 owns those until then); the extractor's candidate and export detection stays, and this decision is about that part.
- A host that addresses a component by display name keeps working: `component_in` stays, and a lookup by export is added beside it.

## Alternatives considered

- Keep the display name and fix each reader: five readers so far, each fixed alone, each with its own normalization.
- Key on the checker's symbol handle: it lives in one snapshot of one checker (ADR 0026 rules out keying on checker ids), and the structural path and the hosts never see it.
- Key on the span across revisions: it moves with every edit above the component, which is why `ComponentId` uses the name and an ordinal.
