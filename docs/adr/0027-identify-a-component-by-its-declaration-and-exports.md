# 0027. Identify a component by its declaration and its exports, not its display name

**Status:** Proposed **Date:** 2026-11-10

## Context

The extractor knows which declaration a component is and which names its module exports it under (`LexedExport::LocalDeclaration { name, declared_as }`, the attached sub-components in `extractor/uses.rs`). It keeps neither: `ComponentMapping` holds `component_name` and `exported: bool` (set by name), and `ComponentEntry` holds `display_name` and `exported`. Every reader that needs to know which export a component is works it out again from the display name:

- The checker backend takes the export named like the display name, or like its last `/` or `.` segment, else the module's `default` (`crates/semantic/src/component.rs`). With `Menu.Item = Item; export default Menu`, the checker run gives `Item` the props of `Menu` and reports it as resolved by the checker (reproduced with the CLI and tsrs 0.23.0: structural `Item` has `value`, checker `Item` has `label` and `open`).
- The merge and the session pair an answer with its entry by file and display name (the merge in `types/semantic.rs`, `pipeline/semantic_hook.rs`). A late answer takes the first component with no pairing at all (`pipeline/semantic_hook.rs`).
- The oracle matches by `normComponent` (`apps/oracle/src/tool-universe.ts`), which strips the path, the `.` prefix and every non-alphanumeric character, so `_Card` and `Card`, or `Menu.Item` and `Item`, are one name. A default and a named export of one function were counted twice (review finding on PR 413).
- The Storybook addon compares a story's import name with `displayName` and reads the module's exports with regular expressions (`packages/storybook/src/exports.ts`).
- The S5 probe prints a generic default as text and reads it in another module, where a module-local `FormData` becomes the DOM global (review finding on PR 410).

Each case was fixed, or is being fixed, in the reader where a review found it. The next reader starts from the display name again. After ADR 0026's S8 the checker's answer is the default output, so a wrong pairing is no longer confined to an opt-in path. The construct suite has one component per file (`Row`), so S7's gate cannot see one.

## Decision

A component's identity is its declaration and its exports. Its display name is for display and for addressing by a host; nothing pairs, dedupes or caches on it. Two identities serve two purposes, and each is named for its own:

- The extractor records each component's exports: the names its module exports it under (`default` included), or for an attached sub-component the path through its exports, which starts at the export name, not the parent's local name. `Menu.Item = Item` beside `export default Menu` is `default.Item`. A reader maps `default` to its own import binding. `exported` is whether there are any.
- Pairing a checker answer with its entry is by output key: a request carries the key, the answer and each fallback carry it back, and the merge finds the entry by key. Dedupe is by declaration (file and span of the declaring node). The extractor does not record the span in its output yet; the oracle dedupes by declaration today (PR 413), and core does not. Across revisions the identity stays `ComponentId`, because spans move with edits.
- The checker backend asks for a recorded export by its exact name, follows aliases, and keeps the structural result with a reason when the export is missing or declared in another file. It never falls back to `default` unless `default` is a recorded export. A generic sub-component reached through a parent's property is not instantiated; it keeps the structural result with a reason.
- An attachment records which component it attaches by that component's own identity: `Local(name)` for a module binding, `Inline(span_start)` for an inline function with no binding. Nothing re-finds a mapping by display name, so `A.Item = …` beside `B.Item = …` stay two components.
- The canonical JSON gives each component its `exports`. Readers outside the crate (oracle, addon, scripts) join on file and export name and stop re-deriving names. Until part 2 (trail 302) `ComponentEntry` keeps `exported` beside `exports`, and `exports` is not serialized.
- A type argument written into a probe resolves to the declaration the checker read it from: it is either written in the declaring module or checked against that declaration. Printed text is not re-read in another scope unchecked.

## Consequences

- `SourceData` changes, so `CACHE_SCHEMA_VERSION` moves (ADR 0003). The canonical JSON gains a field, so `SCHEMA_VERSION` moves from 6 to 7 with the oracle's `SUPPORTED_SCHEMA_VERSION`, the TypeScript declarations and the `schema` command (ADR 0017). `SemanticTarget`, `SemanticComponent` and `SemanticFallback` are public in `caliper-docgen-core`, so the core crate takes a minor bump.
- The checker's symbol handle names a declaration by node index, kind and path, not by offset, so the backend checks the file, not the span. It compares the file against the module symbol's own declaration path, not a path the host built, because the checker lowercases paths on macOS. A span check would need a request this audit did not find in the protocol.
- The construct suite gains rows with several components in one file: a default and a named export of one function, `export { _X as X }`, an attached sub-component beside a default export, and a default export beside a named one. They gate this change and S7.
- This lands before S7. S7 deletes most of the structural resolver, where the remaining name-capture findings live (ADR 0025 owns those until then); the extractor's candidate and export detection stays, and this decision is about that part.
- A host that addresses a component by display name keeps working: `component_in` stays, and a lookup by export is added beside it.

## Alternatives considered

- Keep the display name and fix each reader: five readers so far, each fixed alone, each with its own normalization.
- Key on the checker's symbol handle: it lives in one snapshot of one checker (ADR 0026 rules out keying on checker ids), and the structural path and the hosts never see it.
- Key on the span across revisions: it moves with every edit above the component, which is why `ComponentId` uses the name and an ordinal.
