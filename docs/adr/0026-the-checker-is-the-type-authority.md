# 0026. The checker is the type authority; oxc supplies syntax

**Status:** Accepted (2026-11-08) **Date:** 2026-11-07

## Context

ADR 0006 and 0007 made the TypeScript checker an opt-in second resolver, because a whole-project `tsc` run cost up to 9.3 s and 975 MiB (`docs/type-checker-integration.md`); ADR 0022 bounded it to a warm per-component request. The structural default still answers wrong without a marker: a type that is only a call signature becomes `{}`, `Omit`, `Pick` and `Partial` reached through an alias lose their effect, and a factory's props are taken from a name-matched interface (`docs/rdt-differences.md`, classes 4 to 6). A prototype found more: `[string, ...number[]]` prints as `[string, number[][]]` and a getter reads as a method. Each fix is another rule in a resolver that re-implements the type system.

tsrs (`oxc-project/tsrs`, Apache-2.0) is a Rust port of the TypeScript 7 checker at microsoft/TypeScript `3e9236361fa9`, the commit `typescript@7.1.0-beta` was built from. It speaks the same `--api` protocol: the 19 methods caliper calls are all present, and 87 one-prop fixtures got the same oracle verdicts as `typescript-beta`. A throwaway prototype through the Node client (not in the repository; these numbers are the record) measured on Chakra `ButtonProps` (1,138 props) a 43-47 ms snapshot and the first converted prop at 62-67 ms, where ADR 0022 measured 703 ms with `tsc` on a different program root (indicative, not an A/B), with tsrs peaking at 126-130 MiB. On 16 hand-written components the checker was equal or better on 14, worse on union member order and an unresolvable import. A polymorphic component instantiated through a virtual file gave 267 props for `'div'` where both engines give 2 today. tsrs's own benchmark (`bench/RESULTS.md`) reports 1.8 to 4.2 times TypeScript 7.1 beta's speed and 2.1 to 2.6 times less memory.

## Decision

The checker decides types and oxc decides syntax.

- **Authority.** The checker is the default source of a component's prop set, each prop's type, requiredness and description. oxc supplies candidate components (`extractor/evidence.rs`, `uses.rs`), defaults, JSDoc positions, source locations and declaration order, which is re-applied to union members because the checker sorts them by flags and value.
- **One projection model.** Checker facts convert once into the canonical model. Canonical JSON, TOON and the RDT view are projections of it, and no projection makes a type decision.
- **Fallback.** The structural resolver stays only for when the checker cannot answer: not installed or offline, an unsupported platform, a crash, a timeout. Its types carry `degradedBy` and the run carries one diagnostic.
- **Opt-out.** The checker runs unless `--no-checker` or a config option turns it off; `--experimental-semantic-types` and `semanticTypes` give way to it.
- **Per surface.** The Storybook addon keeps ADR 0022: the structural answer first, the checker's within `semanticWaitMs`. Its premise that the checker is too slow for first paint was re-measured with tsrs in S4 and holds (Chakra's `Button`, 562 ms, 402 ms after the first-answer work). The CLI and agents wait for the checker, all candidates in one snapshot, with a timeout (10 s proposed).
- **Pin.** We pin an exact tsrs build (`0.23.0-ts7.1.0-beta` today) and bump it on purpose. A bump passes the deterministic construct suite (`apps/oracle/construct-suite`, being built on `feat/construct-suite`) and a tsrs-against-Go parity run of it. The weekly oracle keeps the Go `typescript` checker: an oracle running the checker that produced the answer agrees with it by construction.
- **Process.** The checker process is replaced above an RSS threshold, as `OnDemandConfig::max_rss_mib` does for `tsc`. `tsrs` stays user-installed (or `--tsc-path`) while the checker is opt-in. It becomes an optional dependency of the npm packages (`@tsrs/<platform>`) at S8, with the default flip, after the install smoke tests measure its size (23 to 50 MB per platform).
- **Collapsed React types.** `ReactElement`, `JSX.Element` and `ReactChild` stay collapsed to the `reactNode` terminal. No evaluation showed it hurting Storybook controls.

Not pursued: linking tsrs in-process through `tsrs_api::Session`. The crates are `publish = false` (crates.io request #858 has no maintainer answer), process isolation is lost (a checker abort would take down the Storybook dev server), and the memory growth would live inside the addon with no restart lever. It can be revisited if the crates are published and an isolation story exists.

## Consequences

- Supersedes "structural stays the default" in ADR 0006 and 0007 and 0007's "never a silent default" trigger. The rest of both stands.
- Deletable or shrinkable with a thin fallback, estimated from each file's header: in `crates/core/src/resolver`, `alias`, `chain`, `evaluate`, `substitute`, `named`, `primitives`, `extends`, `func`, `template`, `import`, `callee`, `memo`, `deps`, `collected` and `precedence` shrink to a no-generics subset and `mod.rs` by half; `dependency_types.rs` and `import_map.rs` go (the checker resolves modules; ADR 0010 and 0025 narrow to the fallback); in `extractor`, `interface` (but cva), `alias`, `values`, `reexports`, `asserted` and most of `visit` go, and `component`, `wrapper` and `mod.rs` shrink to candidate detection. About 7,600 of 25,300 non-test lines, 9,400 with no fallback, which Windows ARM64 rules out. `html`, `react`, `ambient`, `evidence`, `uses`, `defaults` and `jsdoc` stay; `crates/semantic` and `crates/tsc-client` grow and `convert.rs` is rewritten.
- Contract: `PropType` gains what the checker knows and it cannot hold: tuple element optional, rest and labels, `bigint`, `unique symbol`, enum identity, template literals, overload sets, call signatures on objects, a recursion marker, and a typed truncation in place of `Named` with checker text. React terminals (`ReactNode`, `CSSProperties`, `Ref`, `ElementType`, HTML attributes) are recognized by declaring path, as `is_react_declared` (`crates/semantic/src/component.rs`) does. `SCHEMA_VERSION` and `CACHE_SCHEMA_VERSION` bump (ADR 0017).
- No cache is keyed on a type id: ids are per checker and change with query order. Keys are file content, export name, instantiation and the tsrs pin. The checker does not see an edit it is not told about, so watch forwards every change.
- Unsolved: a per-component dependency closure from the checker (invalidation stays conservative), and the structural answer's gaps wherever the fallback runs.
- Risks: tsrs is pre-1.0, four releases in twelve days, and closed to pull requests (its README). Memory grew from 98 to 246 MB over 150 Chakra edits with each old snapshot disposed (#920 and #646 open). Determinism across checker counts is shown only on a 72-file program. There is no `@tsrs/win32-arm64` package, so those users get the fallback. Binaries are 23 to 50 MB unpacked per platform.

Migration, each slice behind a deterministic gate, none on the oracle. The tsrs backend (the Rust client running on tsrs, PR 390) is done.

1. **S1** (done) Shared projection model and the new `PropType` variants, filled by the structural path where it knows them. Gate: a unit test per variant, golden JSON diff, cache schema test.
2. **S2** (done) Flag-based converter in `convert.rs`, memoized by type id within one snapshot, with a named-boundary policy and caps. Gate: the construct suite's expected JSON; reversed query order gives identical bytes.
3. **S3** (done) React terminals by declaring path. Gate: no regression against today's Chakra `Button` terminals.
4. **S4** (done, one gate failed) The checker's answer wins when it runs: the addon inside ADR 0022's window, the CLI waiting (`--semantic-timeout-ms`, 10 s), and a prop the checker could not read keeps its structural type with `degradedBy` (`docs/type-checker-integration.md`). Gate: checker-on output equals structural on the suite except a listed set of improvements (held by the checker lane); Chakra first answer at most 150 ms through the Rust client (measured 562 ms, then 402 ms once each answer was parsed once and a type's parts asked for once; FAIL: the floor of that request set with no round trips is about 230 ms, 81% of its bytes the property lists of Chakra's style-prop condition maps, so the seam has to change rather than the budget: [gate measurements](../gate-measurements.md#first-answer-s4), and ADR 0022's structural-first answer stands); the Storybook harness re-run (not run: it needs the corpora).
5. **S5** (done) Instantiation of generic and polymorphic components through a virtual file: a component whose first type parameter declares a default is read at it, from a file the checker holds in memory (`createSnapshot`'s `fileSystem` layer, read by tsrs and the Go beta alike) in a second snapshot made only when a requested component is generic; an instantiation that does not form keeps the structural props with the checker's error as the reason ([generic components](../type-checker-integration.md#generic-components-adr-0026-slice-s5)). Gate: `Poly<'div'>` gives at least 260 props and `'a'` differs (PASS: 281 and 289, on tsrs and the Go checker, `crates/semantic/tests/instantiate.rs`).
6. **S6** Watch on `snapshot.update` with RSS recycling. Gate: an unnotified edit fails a test loudly; Chakra edit-and-query p95 at most 25 ms; 300 Chakra edits stay under the threshold.
7. **S7** Delete the structural resolver for what the suite covers. Gate: every suite row passes with the fallback off on supported platforms; a Windows ARM64 job runs the fallback.
8. **S8** Make the checker the default (`--no-checker`), and the docs. Gate: `just lint doc-check machete typos`.

### Gates before the default flips

S8 stays behind these gates:

- The checker lane of the construct suite (caliper's checker path over `apps/oracle/construct-suite`) shows no regression against the structural path except listed improvements.
- Output is deterministic across checker counts on a large program.
- A Linux CI run of the checker path is green.
- Memory behaviour past 600 edits in a watch session, with the recycle threshold (768 MiB `max_rss_mib`) exercised.
- The tsrs-against-Go parity check (`pnpm --filter caliper-docgen-oracle construct:parity`, PR 392) passes on the pinned version, and runs on every pin change.

The determinism and memory gates are measured by `just gate-measure`; what it reads, and why the first gate's wording cannot be met as written (tsrs's `--api` has no checker-count option), is in [gate measurements](../gate-measurements.md).

## Alternatives considered

- **Keep the checker opt-in.** Two sources of truth, and the default keeps the silent wrong answers above.
- **Checker for the addon only.** The CLI and agents read the same wrong types, and they can afford to wait.
- **Link the tsrs crates in-process now.** Unpublished crates, a toolchain change and a pre-1.0 internal API, before the process route is measured through the Rust client.
- **Parse with `tsrs_parser` instead of oxc.** Unpublished like every tsrs crate, and the extractor is written against oxc's AST, which already gives the syntactic facts kept here at about 2 ms per incremental update.
- **Keep investing in the structural resolver.** Each defect class above re-implements a checker rule and was silent until a comparison found it.
