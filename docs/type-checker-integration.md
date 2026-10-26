# Semantic backend (TypeScript 7)

**Status:** slice 1 implemented as an experimental opt-in, off by default ([ADR 0006](adr/0006-semantic-backend-as-second-resolver.md), [ADR 0007](adr/0007-backend-discovers-components-and-compiles-in.md)): `extract --experimental-semantic-types` (see "Slice 1" below). Structural extraction stays the default and the fallback. `PipelineOptions::resolve_complex_types` exists and does nothing. Separately, TypeScript 7.1's checker runs as a test-only oracle for structural prop types (`scripts/parity.py oracle`, `docs/parity.md`). **Last verified:** 2026-10-23 for slice 1 against `7.1.0-beta`; 2026-09-27 for the Cost measurements, each naming its nightly; 2026-10-22 for the API table against the `7.1.0-beta` package; 2026-10-23 for the fixture-corpus parity on `7.1.0-beta`.

## Next step

Slice 1 below covers function components by their export. Next, in order: carry what `PropType` cannot (optional and rest parameters, index signatures, `readonly`), per-prop `tags` and `parent`, class and generic components, `batchRequests` to cut round trips, watch-mode notifications, then the NAPI binding. The client's open Lifecycle items in the Gates table still stand.

Some props can't be resolved from the AST alone. An opt-in backend asks TypeScript's checker instead, using the algorithm react-docgen-typescript (RDT) uses, and merges the answer into the structural result. Structural extraction stays the default.

## What the structural resolver can't do

Component prop counts from `fixtures/` with `--html-attributes full`, against RDT. Root causes are from `docs/benchmarks.md`.

| Gap | Example | Ours | RDT |
| --- | --- | --: | --: |
| Generics through alias chains | ariakit `Props<T, O> = O & Omit<ComponentPropsWithoutRef<T>, keyof O>` | 5 | 294 |
| Generics through alias chains | ark-ui `SelectRoot` | 36 | 314 |
| Conditional types | headlessui `PropsOf<TTag>` | 14 | 246 |
| Mapped types over a generic | base-ui `WithBaseUIEvent<T>` | 11 | 293 |
| Conditional type and indexed access | fluentui `Slot<Type, ...>` | 8 | 303 |
| `@emotion/styled` two-argument overload | storybook-emotion `Button` | 7 | 306 |

In the 2026-07-25 baseline, 10 of 33 compared components account for 2,696 of 2,720 missing props. The other 24 are `ref` and `key`, which this tool omits by design (`MIGRATING.md`).

## How it works

```
OXC parse → structural resolver ──────────────┐
                                              ├→ merge → ExtractionOutput
tsc --api → semantic resolver (opt-in) ───────┘
```

The structural result is complete on its own and returns first. The semantic pass reads each exported component from the checker and replaces the structural props where it finds one.

### Merge rules (proposed)

| Field | Source |
| --- | --- |
| Props present, `type`, `required` | Semantic |
| `parent`, `declarations` | Semantic. `declarations` lists every declaring interface, not just the first |
| `description`, `tags` | Semantic JSDoc, structural if empty |
| `defaultValue` | Structural. RDT reads defaults from the AST too |
| Components tsc doesn't surface (not exported) | Structural only |
| Degrade diagnostics of a resolved component | Dropped |

### What gets dropped, and where

| Dropped | Where | Status |
| --- | --- | --- |
| `children` without a description | RDT projection only. Canonical keeps it | Done |
| Ambient `.d.ts` components such as `PureComponent` | Merge step | Done |
| `ref`, `key` | Not emitted, matching the structural path | Open |
| HTML and DOM attributes in curated and none mode | Semantic adapter, by declaring interface rather than by name | Open |

### Delivery (proposed)

The structural result returns immediately. The semantic result follows: the CLI waits for it behind an opt-in flag, and the NAPI binding exposes a second async call so the Vite plugin can patch when it resolves. Spawn `tsc` at the start of the run so it loads while extraction runs. Each result carries a content hash, and a stale one is dropped.

## Slice 1 (experimental, off by default)

`caliper-docgen extract --experimental-semantic-types [--tsc-path PATH]`, or `experimentalSemanticTypes` and `tscPath` in `docgen.config.ts`. `crates/semantic` spawns the project's `node_modules/typescript/bin/tsc --api` (version pinned to `7.1.0-beta`), builds one program over the files of the components the structural pass found, and asks for each exported function component its call signature's first parameter and that type's properties. The CLI then calls core's `merge_semantic`. Not wired into watch, `check`, `inspect` or the NAPI binding.

- **Merge:** props present, `type` and `required` from the checker; `defaultValue`, `parent`, `declarations`, `tags`, inheritance layers and curated inherited props stay structural. The checker is asked for a prop's description only when the structural pass has none (65% of Mantine's requests otherwise). Props `@types/react` declares are excluded by declaring file. Identical merged props are shared, as in the structural result.
- **Output type:** no new `PropType` variant and no schema change. Primitives, literals, unions (with `boolean` collapsed and `undefined` dropped from optional props), arrays, tuples, inline objects of up to 16 fields and single-signature functions map onto existing variants; `ReactNode`, `ElementType`, `CSSProperties` and `ReactElement` map to their terminals; anything else (named interfaces, enums, generics, overloads, anything past depth 4) is `Named` with the checker's own text. Union members keep the checker's order, which is creation order, not source order.
- **Provenance:** `Info` diagnostics with code `SEMANTIC_BACKEND` whose `affects` lists the output keys: one for the components the checker resolved, one per fallback reason. A first-class per-component field would change the canonical JSON and needs an output schema version first.
- **Fallback:** a missing, unpinned, crashed or timed-out `tsc` gives the structural result plus one `SEMANTIC_BACKEND` warning, never an error. Per component, the structural result stands for: not exported, class or non-function, overloaded, no props parameter, `any`/`unknown`/generic/union props type, export not found by name (a display name such as `@mantine/core/Accordion` also tries its last segment), or a request the checker rejected. Each is listed with its reason.
- **Not carried:** optional and rest parameters, index signatures, `readonly`, generic parameters of functions, per-prop `tags`, `parent`/`declarations` from the checker. Default exports match by `default` when no export has the component's name.
- **Tests:** `crates/semantic/tests/replay.rs` runs the backend against `replay-tsc`, which answers from a recording of the real `7.1.0-beta` (`tests/fixtures/recording.json`, refreshed by `cargo run -p caliper-docgen-semantic --example record_fixture -- <typescript package dir>`); a request the recording lacks fails the test. Fallbacks (no `tsc`, unpinned version, bad path, crash, hang) are asserted as the structural output plus one warning. The CLI test asserts the default output has no `SEMANTIC_BACKEND` diagnostic.

Measured on `7.1.0-beta`, `extract --json` (best of 2 to 3), flag on against off, plus the base commit's output compared byte for byte (identical apart from `durationMs` on all five corpora, with and without `--load-dependencies`). Peak RSS is the larger of the two processes; on Chakra Caliper is 117 MiB and `tsc` 945 MiB.

| Corpus   | Components resolved by the checker |            Off |              On |
| -------- | ---------------------------------: | -------------: | --------------: |
| mantine  |                          332 / 344 |  47 ms, 82 MiB | 588 ms, 287 MiB |
| chakra   |                          598 / 598 |  38 ms, 68 MiB |  9.3 s, 975 MiB |
| shadcn   |                      1,693 / 1,884 |  48 ms, 81 MiB |  1.5 s, 874 MiB |
| antd     |                          206 / 248 |  36 ms, 69 MiB | 508 ms, 443 MiB |
| spectrum |                      1,153 / 1,356 | 71 ms, 118 MiB | 618 ms, 381 MiB |

Chakra's time is `tsc` computing 470,000 shared props' types, not round trips. Before shared props were interned in the merge, Caliper itself held 7.4 GiB on Chakra.

Against the oracle (`scripts/parity.py oracle`, flag on, `7.1.0-beta`), wrong verdicts fall to 0.0% on every corpus but antd (0.1%), but the identical-or-spelled share also falls: mantine 91.3% to 22.0%, chakra 100% to 16.4%, shadcn 85.2% to 52.3%, antd 91.9% to 82.3%, spectrum 83.2% to 28.3%. The oracle credits the structural result for keeping a type's name (`StyleProp<MantineSpacing>`); the checker's answer is expanded to its members and compared as "less precise". Keeping the alias name and arguments needs a `PropType` that carries them, which is an output change.

## On demand, per component (experimental, off by default)

A long-lived session (the Storybook addon, the lazy `WatchSession`) can attach a warm checker instead of running the whole-project pass above: `semanticTypes: 'on-demand'` (ADR 0022). `caliper-docgen-semantic` keeps one `tsc --api` process on its own thread, builds a program rooted at only the requested component's file, and returns its props. `WatchSession::component` merges them over the structural answer when they arrive within `semanticWaitMs` (default 150 ms); otherwise it returns the structural answer plus an `Info` diagnostic, and the checker's late answer serves the next request of the same, unedited file.

Measured with `crates/semantic/examples/ondemand_bench.rs` (50 random components, TypeScript 7.1.0-beta): the first answer takes 82 to 137 ms (Chakra 703 ms), a component asked for alone 21 to 41 ms (Chakra 534 ms), and 2 to 9 ms when asked for in batches of 8 to 32. Where the structural pass is behind (shadcn) the checker reports 2.3 times the props; where it is not (Mantine, Chakra, Spectrum) it reports the same count. The in-memory cache, the 64-program process replacement and the failure modes are in the ADR.

## The algorithm

Mirrors RDT 2.4.0 (`lib/parser.js`). Method names are from the 7.1 client.

1. `getSymbolOfSourceFile(file)`, then `getExportsOfModule`.
2. For each export, `getTypeOfSymbol`, then `getSignaturesOfType(Call)`. A component has a signature whose first parameter is named `props` or is the only parameter. For a class, use `Construct` signatures and read `props` from the instance type.
3. Props type: `getTypeOfSymbolAtLocation(param, param.valueDeclaration)`. Props: the type's apparent properties, plus each member's when it is a union.
4. Per prop, `type` is `typeToString(getBaseConstraintOfType(t) ?? t)` with default flags, so long types truncate as they do in RDT. `required` means the symbol lacks `Optional` and has no code default.
5. `parent` is the first declaration's parent when that is an interface or type alias. `declarations` is every declaration's parent.
6. `description` and `tags` come from `getDocumentationComment` and `getJsDocTags`.

RDT also emits any exported symbol that has a JSDoc comment but no props. We don't. In the fixture corpus, 23 of the 26 entries only RDT emits are of this kind, such as enum members, type aliases and functions.

## TypeScript 7.1 API

| Topic | Detail |
| --- | --- |
| Where | `microsoft/TypeScript` `main` (`7.1.0-dev`). typescript-go closed on 2026-08-20 |
| Timeline | The plan ([#63703](https://github.com/microsoft/TypeScript/issues/63703)) put the beta on 2026-10-06; npm published `7.1.0-beta` on 2026-10-20. RC 2026-11-10 and stable 2026-11-24 are still planned |
| Get it | `npm i typescript@beta` (`7.1.0-beta`) or `@next` (daily `7.1.0-dev`). 7.0.x (`latest`, `7.0.2`) has no API |
| Launch | `tsc --api [--async] --cwd <dir> --useCaseSensitiveFileNames=<bool>`, plus `--runExternalCode` and `--timing` when asked. With `--async`: JSON-RPC with LSP-style `Content-Length` framing over stdio (`vscode-jsonrpc`), handled concurrently. Without it: msgpack over stdio |
| msgpack caveats | Each message is a 3-element msgpack array: message type (u8), method (bin) and payload (bin). The server answers one request at a time, so a sync client blocks until the reply. Encoded strings are WTF-8: a lone surrogate from a JS string survives, so a Rust client must not decode them as strict UTF-8. On Windows the sync client passes `--pipe <named pipe>` instead of stdio |
| Stability | The beta's JS entry points are `typescript/sync`, `/async`, `/proto`, `/ast` and `/ast/*`, `/fs`, `/path` and `/vscode`; the root export is only the version. The `unstable/*` names in ADR 0006 were the nightlies' and are gone. The protocol has no version handshake, so pin the build |
| Positions | UTF-16 code units. OXC offsets are UTF-8 bytes |
| Surface | 177 methods in `7.1.0-beta` (`APIMethodInfo` in `dist/api/proto.generated.d.ts`), up from 157 on 2026-09-19, mostly with TS 6 checker names. Every call in "The algorithm" is present |

**Beta re-probe (2026-10-21, `7.1.0-beta`).** `crates/tsc-client`'s smoke example spawned the beta's `bin/tsc --api`, completed the `initialize` round trip (`caseSensitivity`, `currentDirectory`) and shut down cleanly with exit status 0, so framing and shutdown behave as the fake-tsc harness assumes. `TSC_PACKAGE_DIR=<dir> cargo run -p caliper-docgen-tsc-client --example smoke_real_tsc` repeats it against any installed `typescript`. The beta's `proto.generated.d.ts` still exposes every checker call the algorithm uses: `getTypeAtLocation`, `getTypeOfSymbol`, `getDeclaredTypeOfSymbol`, `getSignaturesOfType`, `getResolvedSignature`, `getReturnTypeOfSignature`, `getBaseTypes` and `getPropertiesOfType`. The fixture-corpus parity run below was re-run on the beta on 2026-10-23 (`apps/validate` now imports `typescript-beta`, `npm:typescript@7.1.0-beta`, since the `unstable/*` entry points are gone). Two beta behaviors to know: the server panics (`interface conversion: interface {} is string, not float64`) when `createProgram` gets string-valued enum options such as `target: 'es2022'`, so pass options from `parseConfigFile`; and `true`/`false` literal types carry neither `value` nor `intrinsicName`, so read them with `typeToString`. Discovery over a whole project ran in the oracle: 33 to 1,531 components per corpus.

**Churn.** About 20 commits touched the protocol definition (`proto.go`) between 2026-08-20 and 2026-09-18. `updateSnapshot`, project IDs and `createProgram` params were reshaped on 09-17 and 09-18. Every call the algorithm uses ran unchanged on the 09-20 and 09-26 nightlies. Re-verify on the beta.

**Freshness.** tsc sees an edit to a dependency only when told:

```js
const s1 = await s0.update({ fileNotifications: { changed: [file] }, ensurePrograms: true })
```

With the notification, the dependent component's props updated (0.5 ms update, 2.8 ms query). An unnotified disk edit stayed stale (2026-09-20). Both incremental paths must notify it once the backend is wired in: `WatchSession::update_file` and `WatchSession::ensure_fresh`, which finds changes by stat instead of being told (ADR 0007 names only the first).

## Evidence

Method: RDT 2.4.0 on typescript@6.0.3 (`apps/validate`'s RDT is pinned to a TS6 API — see "The RDT oracle" below) with the harness options (`shouldExtractLiteralValuesFromEnum`, `shouldRemoveUndefinedFromOptional`, `propFilter: { skipPropsWithoutDoc: false }`), against the algorithm above via `apps/validate/src/run-semantic.ts` on the nightly named in each row, or `7.1.0-beta`. Run with `pnpm --filter caliper-docgen-validate parity` (`PARITY_SHOW=all` lists every mismatch).

| Check | Result | Nightly |
| --- | --- | --- |
| 13 components: the 10 outliers, base-ui `MenuRoot`, shadcn `Button`, mui `Button` | 0 missing and 0 extra props. `required` 3412/3412. Type strings 3410/3412 | 09-26 |
| Descriptions: ariakit, MUI, fluentui, shadcn | 294/294, 311/311, 288/288, 292/292 | 09-20 |
| `parent`: ariakit `MenuButton` | 294/294 in 12 requests | 09-20 |
| Fixture corpus, 55 files in one program | 70 components in common, equal prop counts on 70/70 | 09-20 |
| Class components | 2/2 on a synthetic file. The corpus has none | 09-20 |
| Full fixture corpus, 44 files / 47 components / 5554 props, one program | 5 missing, 27 extra. `required` 5508/5522. Type strings 5265/5522. Descriptions 47/47 | 09-26 |
| The same run | 5 missing, 27 extra. `required` 5508/5522. Type strings 5266/5522. Descriptions 47/47. Of the 13 components above, the two nested `ref` unions and ark-ui `SelectRoot.ids` differ | `7.1.0-beta` |

Type strings match after two normalizations. tsc prints union members in internal type-id order, which differs from TS 5.9, so members are compared as a set. RDT also runs `.replace(' | undefined', '')` twice, which turns a nested `=> string | undefined` into `=> string` and loses information. Canonical output keeps it. The 2 remaining differences are nested `ref` unions in fluentui and base-ui.

Of the corpus's 26 RDT-only entries, 23 have no props and 3 are default exports RDT names differently (mui `Button` confirmed at 311/311).

The full-corpus run's 257 remaining mismatches, unlike the curated 13, aren't all real algorithm gaps:

- Most are non-component exports the "single-parameter function" heuristic sweeps in — plain string-returning helpers (`antd/buttonHelpers.ts`, `headlessui/render.ts`'s `mergeProps`, `blueprint/blueprintTypes.ts`) whose "props" are `String.prototype`/`Array.prototype` members. RDT is equally permissive and picks these up too, so both tools agree these aren't components; the mismatches are TS 6 vs TS 7.1-dev lib.d.ts differences (e.g. `padStart`'s parameter renamed `maxLength`→`targetLength`) and per-run `Symbol.iterator` id suffixes, not a resolver bug.
- `children` without a description shows as "extra" — RDT drops it, canonical keeps it (see "What gets dropped", status Done).
- One real RDT bug surfaced: `rdt-compat/svg-icon.tsx` declares two `forwardRef` components (`Icon`, `Box`) in one file; RDT resolves `Box`'s `ref` and DOM-attribute types to `Icon`'s `SVGSVGElement` instead of `Box`'s own `HTMLDivElement`. The semantic algorithm gets `Box` right — RDT is the outlier.

## Cost

Measured through the Node client on one macOS machine, so a Rust client should be lower.

| Measure | Result |
| --- | --- |
| Spawn and first snapshot | 33–72 ms |
| First checker query | 1–15 ms |
| Request round trip | 0.1 ms. 500 positions in one request take about 1 ms |
| 55-file corpus, one program | 385 ms and 937 requests. RDT in-process: 955 ms |
| Server memory | About 100 MB (MUI `Button`) |
| Structural path, for comparison | 50 ms for the whole corpus, 1.75–2.1 ms per incremental update (`docs/benchmarks.md`) |

Trimming the process, on the MUI `Button` query (median of 5 runs, 2026-09-20). Differences under about 10 ms are noise.

| Config                                 |   Wall |    CPU | Memory |
| -------------------------------------- | -----: | -----: | -----: |
| Default                                |  85 ms | 270 ms | 102 MB |
| `types: []`                            |  79 ms | 260 ms | 100 MB |
| `types: []`, `GOGC=off`                |  71 ms | 200 ms | 111 MB |
| `types: []`, `GOMAXPROCS=2`            | 114 ms | 160 ms |  99 MB |
| Default, plus 300 unrelated root files |  88 ms | 320 ms | 112 MB |

Pass only the files you need as roots. The API's checker is lazy, and diagnostics and emit never run.

## Boundaries

- **`crates/core`** owns the semantic result type, the merge and the diagnostics. It has no I/O, no async and no protocol types. The merge lives here because `OpaqueDetail` and `ParsedProp` internals are `pub(crate)`.
- **Backend crate** (`crates/semantic`, always compiled in and enabled at run time, ADR 0007) spawns `tsc --api` through `crates/tsc-client`, runs the algorithm, and handles timeouts. Core never depends on it: the CLI runs the backend and calls core's `merge_semantic`.
- **CLI and NAPI** only switch it on. Neither calls JS.
- **Not a `DocgenPlugin`.** The hook is synchronous and runs on the cold path only. `WatchSession::update_file` skips plugins.
- **Failure** leaves the structural result and adds a diagnostic. It never fails extraction.
- **Determinism.** Reversing the query order changed no union order across 16 props in 3 files (2026-09-20). Whether canonical output sorts unions is open (below).

## Gates before it ships

| Gate | Status |
| --- | --- |
| Accuracy against RDT | Passed on 13 components and re-verified on the full 47-component fixture corpus. See Evidence and not verified below |
| Lifecycle | Client built and tested against a fake tsc and the real nightly (`crates/tsc-client`): missing binary, unsupported version, timeout, crash, restart storm, malformed or oversized frame, partial project failure, shutdown reaping, stale notification. Open: header-size cap, stderr that isn't UTF-8, honoring `handshake_timeout`, recovery after the restart cap trips, whether a restarted request may replay |
| Cost | Measured through the Node client. Repeat with the Rust client |
| Determinism | Clean and incremental runs must produce the same output |
| Oracle | Fixed. See below |

## The RDT oracle

`apps/validate` depends on TypeScript 7.1 directly (for `run-semantic.ts`'s own `tsc --api` probe and the oracle), but RDT 2.4.0 crashes on import against it (`ts.JsxEmit` is undefined). RDT now resolves its `typescript` peer to `typescript@6.0.3` instead, via the aliased `typescript-beta` dependency (`npm:typescript@7.1.0-beta`, pinned exactly) the probe and oracle import from — no pnpm override needed, since pnpm overrides don't reliably rewrite a peer dependency's resolution (tried `pnpm-workspace.yaml`'s `overrides: { "react-docgen-typescript>typescript": ... }`; the lockfile recorded it but RDT kept resolving against whatever `typescript` apps/validate depended on directly).

`compare.ts`/`analyze.ts` (caliper-docgen vs RDT/react-docgen) still compare only name, `required`, type and default, and render our types themselves in `run-ours.ts` (objects print as `{ ... }`). Their type mismatches mix resolver and renderer differences, and description, `parent` and `declarations` are never compared. `parity.ts` (RDT vs the semantic algorithm) compares description too.

## Open decisions

1. **Output type.** Checker types don't fit `PropType` losslessly: `Named` has no file or symbol, `Tuple` has no optional or rest elements, and `Object` has no index signatures. Add a variant or map into existing ones. `ExtractionOutput` has no schema version, so add one first.
2. **`ref` and `key`.** RDT emits both, and nothing else records forwardRef metadata. Semantic mode must match whatever the structural path does.
3. **Trigger.** Run on every exported component or only on files with degrade diagnostics. Bare generic parameters and TypeScript utility types resolve to `Named` with no diagnostic by design, so the second misses them.
4. **Union order.** Sort in canonical output, or keep source order for structural results.

## Not verified

- Cost through the Rust client. Every measurement uses the Node client.
- Overloads with more than one signature, static sub-components (`Menu.Item`), and barrel re-exports, where RDT names components by the barrel file.
- `@default` and `@type` JSDoc overrides.
- Projects larger than the 44-file corpus.
- What the `MappedType` accessors added on 2026-09-23 ([#64397](https://github.com/microsoft/TypeScript/pull/64397)) expose beyond resolved properties.
