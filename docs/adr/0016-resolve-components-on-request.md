# 0016. Resolve components on request, and load dependencies for the names their resolution reaches

**Status:** Accepted **Date:** 2026-10-24

## Context

A session resolved every component before it answered one. The Storybook addon, an editor server and `monokl` (which links `caliper-docgen-core` in-process) ask about one component at a time, and a rich answer (`load_dependencies`, full HTML attributes) is what costs: Chakra starts in 340 ms and 460 MB with both on, 40 ms and 68 MB without. That cost is why consumers have kept the thin answer. Parsing and indexing are cheap (30 to 110 ms on the five corpora); resolution and dependency loading are the rest.

## Decision

`WatchSession::lazy(options)` is the library entry point. It is `Send + Sync`, takes no CLI or NAPI crate, and exposes no parser type: OXC 0.154 and `oxc_resolver` 11 are implementation details, so a host that also links OXC need not agree on a version.

- **API.** `initialize()` parses and indexes every file and resolves nothing. `component(key)`, `component_in(file, name)`, `components(keys)`, `docgen_info(key)` and `component_keys()` answer from there; an eager session answers the same calls from its published output. An answer is an owned `ComponentAnswer { key, entry, diagnostics }`. The default eager path, `extract()` and the CLI are unchanged.
- **Parity.** The answer for a key equals the entry a batch extraction publishes under that key, and `diagnostics` equals the diagnostics batch attaches to it, with `affects` reduced to `[key]`. Run-wide diagnostics (parse errors, missing libraries, key collisions) and enums are not part of an answer. `tests/lazy_parity.rs` asserts this for every component of the fixtures and of the five corpora, with and without `load_dependencies` and full HTML attributes, asked all at once and one at a time in reverse order.
- **Memo.** A resolved component is memoized per revision. An edit keeps every entry whose recorded dependencies (`Resolved::affected_by`, ADR 0010's recorder) the change does not touch, and reports the keys of the names it dropped as `updated_keys` and `removed_keys`; the caller asks again. `ensure_fresh` and `rebuild` work as in an eager session.
- **Dependencies by closure.** A request resolves the component, takes the dependency demands of the project files that resolution read, and if the layer lacks any, loads them (`dependency_types::load` on top of the previous layer, ADR 0010) and resolves again. A resolution is memoized only when it found nothing missing, so a memo never holds an answer computed without a name it reached. The layer starts with the names the project takes from `react`, because loading them follows React's imports (csstype), which any component's HTML attributes reach and which a batch run always has.
- **Contexts keep their memos across growth** only where that is sound: an interface chain or member read carries into the next context when every project file it read, and the file it was keyed in, has its demands in the old layer. A growth that adds no declaration keeps the context.
- **Bounds.** At most one `Resolved` per component; a new revision starts from the entries the edit left valid, with a fresh context. The layer only grows within a session, as in watch mode.
- **Keys.** `component_keys()` never loads a dependency: with `load_dependencies` it lists every re-exported imported component, including ones whose package has nothing to show, and `component` answers `None` for those.

## Consequences

- With `load_dependencies` and full HTML attributes, a lazy session starts in 40 to 67 ms where a batch one takes 68 to 352 ms, and holds 89 to 137 MiB after 50 components where batch holds 108 to 532 MiB (table in `docs/performance.md`). The first component pays for the cold memos a batch run amortizes (Chakra: 126 ms). Fifty components asked for one at a time cost about what the whole project costs in a batch on Chakra (566 ms against 352 ms) and less everywhere else; asked for at once they cost 310 ms. Without those options, start is 28 to 85 ms against 39 to 107 ms and a request is under 4 ms.
- A lazy answer depends on which names the project's files demand, not on a type checker. A name a resolution reads that no demand list mentions is loaded by a batch run when some other file mentions it, and by a lazy session only through the component's own closure. Parity testing found two such cases (React's imports; memos computed before a name loaded) and fixed them. Other cases may exist; a parity test over a new corpus finds them.
- Layer growth costs 2 to 7 ms per request that reaches new names.
- Hosts must not hold the answer of a key across an update that reports it in `updated_keys`.

## Alternatives considered

- **Resolve everything on a background thread.** Memory and CPU stay at the batch cost, and the first answer still waits for its component.
- **Resolve inside the resolver, loading a dependency on a miss.** ADR 0010 rejects it: a result computed before a load is wrong and cannot be told apart afterwards. This decision loads between resolutions and discards the one that missed.
