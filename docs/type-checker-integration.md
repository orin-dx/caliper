# Semantic backend (TypeScript 7)

**Status:** designed, not implemented ([ADR 0006](adr/0006-semantic-backend-as-second-resolver.md)). `PipelineOptions::resolve_complex_types` exists and does nothing. **Last verified:** 2026-09-27. Each measurement names its nightly.

## Next step

Start the Rust client for the Lifecycle gate — Accuracy is re-verified and the probe is committed, so this is no longer blocked on unreproducible numbers.

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
| Timeline | Beta 2026-10-06, RC 2026-11-10, stable 2026-11-24 ([#63703](https://github.com/microsoft/TypeScript/issues/63703)) |
| Get it | `npm i typescript@next`. 7.0.x has no API |
| Launch | `tsc --api --async --cwd <dir>`: JSON-RPC with LSP-style `Content-Length` framing, handled concurrently. Without `--async`: msgpack over stdio, strictly serial. `--pipe` uses a socket |
| Stability | JS entry points are `typescript/unstable/*`. The protocol has no version handshake, so pin the build |
| Positions | UTF-16 code units. OXC offsets are UTF-8 bytes |
| Surface | 157 methods on 2026-09-19, mostly with TS 6 checker names |

**Churn.** About 20 commits touched the protocol definition (`proto.go`) between 2026-08-20 and 2026-09-18. `updateSnapshot`, project IDs and `createProgram` params were reshaped on 09-17 and 09-18. Every call the algorithm uses ran unchanged on the 09-20 and 09-26 nightlies. Re-verify on the beta.

**Freshness.** tsc sees an edit to a dependency only when told:

```js
const s1 = await s0.update({ fileNotifications: { changed: [file] }, ensurePrograms: true })
```

With the notification, the dependent component's props updated (0.5 ms update, 2.8 ms query). An unnotified disk edit stayed stale (2026-09-20).

## Evidence

Method: RDT 2.4.0 on typescript@6.0.3 (`apps/validate`'s RDT is pinned to a TS6 API — see "The RDT oracle" below) with the harness options (`shouldExtractLiteralValuesFromEnum`, `shouldRemoveUndefinedFromOptional`, `propFilter: { skipPropsWithoutDoc: false }`), against the algorithm above via `apps/validate/src/run-semantic.ts` on typescript@next. Run with `pnpm --filter @oxc-react-docgen/validate parity`.

| Check | Result | Nightly |
| --- | --- | --- |
| 13 components: the 10 outliers, base-ui `MenuRoot`, shadcn `Button`, mui `Button` | 0 missing and 0 extra props. `required` 3412/3412. Type strings 3410/3412 | 09-26 |
| Descriptions: ariakit, MUI, fluentui, shadcn | 294/294, 311/311, 288/288, 292/292 | 09-20 |
| `parent`: ariakit `MenuButton` | 294/294 in 12 requests | 09-20 |
| Fixture corpus, 55 files in one program | 70 components in common, equal prop counts on 70/70 | 09-20 |
| Class components | 2/2 on a synthetic file. The corpus has none | 09-20 |
| Full fixture corpus, 44 files / 47 components / 5554 props, one program | 5 missing, 27 extra. `required` 5508/5522. Type strings 5265/5522. Descriptions 47/47 | 09-26 |

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
- **Backend crate** (feature-gated, off by default) spawns `tsc --api`, speaks the protocol, and handles timeouts and supervision. `config.rs` has none of these today.
- **CLI and NAPI** only switch it on. Neither calls JS.
- **Not a `DocgenPlugin`.** The hook is synchronous and runs on the cold path only. `WatchSession::update_file` skips plugins.
- **Failure** leaves the structural result and adds a diagnostic. It never fails extraction.
- **Determinism.** Reversing the query order changed no union order across 16 props in 3 files (2026-09-20). Whether canonical output sorts unions is open (below).

## Gates before it ships

| Gate | Status |
| --- | --- |
| Accuracy against RDT | Passed on 13 components and re-verified on the full 47-component fixture corpus. See Evidence and not verified below |
| Lifecycle | Not started: missing binary, unsupported version, timeout, crash, restart storm, malformed or oversized frame, partial project failure, shutdown reaping, stale notification |
| Cost | Measured through the Node client. Repeat with the Rust client |
| Determinism | Clean and incremental runs must produce the same output |
| Oracle | Fixed. See below |

## The RDT oracle

`apps/validate` depends on `typescript@next` directly (for `run-semantic.ts`'s own `tsc --api` probe), but RDT 2.4.0 crashes on import against it (`ts.JsxEmit` is undefined). RDT now resolves its `typescript` peer to `typescript@6.0.3` instead, via the aliased `typescript-next` package dependency `run-semantic.ts` imports from — no pnpm override needed, since pnpm overrides don't reliably rewrite a peer dependency's resolution (tried `pnpm-workspace.yaml`'s `overrides: { "react-docgen-typescript>typescript": ... }`; the lockfile recorded it but RDT kept resolving against whatever `typescript` apps/validate depended on directly).

`compare.ts`/`analyze.ts` (oxc-react-docgen vs RDT/react-docgen) still compare only name, `required`, type and default, and render our types themselves in `run-ours.ts` (objects print as `{ ... }`). Their type mismatches mix resolver and renderer differences, and description, `parent` and `declarations` are never compared. `parity.ts` (RDT vs the semantic algorithm) compares description too.

## Open decisions

1. **Output type.** Checker types don't fit `PropType` losslessly: `Named` has no file or symbol, `EventHandler` has no return type, `Tuple` has no optional or rest elements, and `Object` has no index signatures. Add a variant or map into existing ones. `ExtractionOutput` has no schema version, so add one first.
2. **`ref` and `key`.** RDT emits both, and nothing else records forwardRef metadata. Semantic mode must match whatever the structural path does.
3. **Trigger.** Run on every exported component or only on files with degrade diagnostics. Bare generic parameters and TypeScript utility types resolve to `Named` with no diagnostic by design, so the second misses them.
4. **Union order.** Sort in canonical output, or keep source order for structural results.

## Not verified

- A Rust client. Every measurement uses the Node client.
- Overloads with more than one signature, static sub-components (`Menu.Item`), and barrel re-exports, where RDT names components by the barrel file.
- `@default` and `@type` JSDoc overrides.
- Projects larger than the 44-file corpus.
- What the `MappedType` accessors added on 2026-09-23 ([#64397](https://github.com/microsoft/TypeScript/pull/64397)) expose beyond resolved properties.
