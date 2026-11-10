# Semantic backend (TypeScript 7)

**Status:** slice 1 implemented as an experimental opt-in, off by default ([ADR 0006](adr/0006-semantic-backend-as-second-resolver.md), [ADR 0007](adr/0007-backend-discovers-components-and-compiles-in.md)): `extract --experimental-semantic-types` (see "Slice 1" below). [ADR 0026](adr/0026-the-checker-is-the-type-authority.md)'s slices S1 to S4 are in: when the checker runs, its answer wins prop by prop (see "The checker's answer wins"); it becomes the default in S8. Structural extraction stays the default and the fallback. Separately, TypeScript 7.1's checker runs as a test-only oracle for structural prop types (`scripts/weekly.py --oracle`, `docs/measurement.md`). **Last verified:** 2026-10-23 for slice 1 against `7.1.0-beta`; 2026-09-27 for the Cost measurements, each naming its nightly; 2026-10-22 for the API table against the `7.1.0-beta` package; 2026-10-23 for the fixture-corpus parity on `7.1.0-beta`.

## Next step

Slice 1 below covers function components by their export. Next, in order: set what `PropType` now carries (index signatures, `readonly`, a ref type's name; optional and rest parameters are done), per-prop `tags` and `parent`, class components (generic ones are read at their default: "Generic components" below), and `batchRequests` to cut round trips; watch-mode notifications are done ("Watch" below). The NAPI binding already ships: a session asks a warm checker for the requested component (`semanticTypes: 'on-demand'`, ADR 0022). The client's open Lifecycle items in the Gates table still stand.

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
| Props present, `type`, `required` | Semantic. A `type` the checker could not read (an `opaque` part) where the structural one is read in full stays structural, marked (see "The checker's answer wins") |
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
| `key` | Not emitted, matching the structural path | Closed |
| `ref` | Emitted in `curated` and `full` for `ComponentProps<"div">`-style props and `forwardRef` components, matching the structural path | Closed |
| HTML and DOM attributes in curated and none mode | Semantic adapter, by declaring interface rather than by name | Open |

### Delivery (proposed)

The structural result returns immediately. The semantic result follows: the CLI waits for it behind an opt-in flag, and the NAPI binding exposes a second async call so the Vite plugin can patch when it resolves. Spawn `tsc` at the start of the run so it loads while extraction runs. Each result carries a content hash, and a stale one is dropped.

## Slice 1 (experimental, off by default)

`caliper-docgen extract --experimental-semantic-types [--tsc-path PATH]`, or `experimentalSemanticTypes` and `tscPath` in `docgen.config.ts`. `crates/semantic` spawns the project's `node_modules/typescript/bin/tsc --api` or `node_modules/tsrs/bin/tsrs --api` (see "Supported binaries"), builds one program over the files of the components the structural pass found, and asks for each exported function component its call signature's first parameter and that type's properties. The CLI then calls core's `merge_semantic`. Not wired into watch, `check`, `inspect` or the NAPI binding.

- **Merge:** props present, `type` and `required` from the checker; `defaultValue`, `parent`, `declarations`, `tags`, inheritance layers and curated inherited props stay structural. The checker is asked for a prop's description only when the structural pass has none (65% of Mantine's requests otherwise). Props `@types/react` declares are excluded by declaring file. Identical merged props are shared, as in the structural result.
- **Output type:** no new `PropType` variant and no schema change. Primitives, literals, unions (with `boolean` collapsed and `undefined` dropped from optional props), arrays, tuples, inline objects of up to 16 fields and single-signature functions map onto existing variants; `ReactNode`, `ElementType`, `CSSProperties` and `ReactElement` map to their terminals; anything else (named interfaces, enums, generics, overloads, anything past depth 4) is `Named` with the checker's own text. Union members keep the checker's order, which is creation order, not source order.
- **Names:** a symbol's `name` from the checker is its escaped name: a name that starts with two underscores has a third (`__size` arrives as `___size`, TypeScript's `escapeLeadingUnderscores`, so that it cannot collide with internal names such as `__call`). `rpc.rs` removes it when the answer is read, so every name downstream is the one the source writes.
- **Error type:** a name the checker cannot resolve (an import that does not resolve, a name declared nowhere) is typed with its error type, which carries the flags of `any`. Only `Type.intrinsicName` tells it from a written `any`: `"error"` against `"any"`, and the error type has an id per site, so the `any` id says nothing. `rpc.rs` reads `intrinsicName`; `convert.rs` turns an `"error"` type into an `opaque` type with the reason `unresolvableImport` (its `specifier` is the name as the source writes it, since the checker does not say which module was behind it), so the prop has `degradedBy: ["unresolvableImport"]` and a written `any` stays `any` with none. No new diagnostic code.
- **Provenance:** `Info` diagnostics with code `SEMANTIC_BACKEND` whose `affects` lists the output keys: one for the components the checker resolved, one per fallback reason. A first-class per-component field would change the canonical JSON and needs a `SCHEMA_VERSION` bump (`crates/core/src/types/output.rs`).
- **Fallback:** a missing, unsupported-version, crashed or timed-out `tsc` gives the structural result plus one `SEMANTIC_BACKEND` warning, never an error. Per component, the structural result stands for: not exported, class or non-function, overloaded, no props parameter, `any`/`unknown`/union props type, a generic component whose instantiation at its default could not be formed (see "Generic components"), export not found by name (a display name such as `@mantine/core/Accordion` also tries its last segment), or a request the checker rejected. Each is listed with its reason.
- **Parameter markers:** the checker's parameter symbols carry no optional or rest flag (an optional parameter has the same flags and `checkFlags` as a required one), so `convert.rs` reads them from what the checker prints for the function type: `name?: T` is optional and its `undefined` is dropped from the type, as for an optional prop, `...name: T[]` is the rest parameter; the signature's `flags` (`HasRestParameter`, bit 1) say the last parameter is a rest parameter either way. A parameter whose type is written `T | undefined` stays required. The type is printed with `typeToString` flags `NoTruncation | InTypeAlias` (plus the checker's own defaults): an alias at the top (`type Handler = (value?: string) => void`) is printed as the parameter list it stands for, not as `Handler`, and nothing is cut short. A `this` parameter is printed but is not one of the signature's parameters, and an overload set or a function with properties is printed as an object type whose signatures are read in order. When the printed type still does not give one entry per parameter (a callable interface prints as its name whatever the flags), a parameter is never taken to be required: the type is `opaque` with reason `unsupportedExpression`, so the prop carries `degradedBy`.
- **Not carried:** index signatures, `readonly` and ref names (`PropType` has them since `SCHEMA_VERSION` 4; `convert.rs` doesn't set them), generic parameters of functions, per-prop `tags`, `parent`/`declarations` from the checker. Default exports match by `default` when no export has the component's name.
- **Tests:** `crates/semantic/tests/replay.rs` runs the backend against `replay-tsc`, which answers from a recording of the real `7.1.0-beta` (`tests/fixtures/recording.json`, refreshed by `cargo run -p caliper-docgen-semantic --example record_fixture -- <typescript package dir>`; the `markers` fixture project, for names and parameters, records to `tests/fixtures/markers.recording.json` with the fixture name as a second argument; the `errors` fixture records the error type); a request the recording lacks fails the test. Fallbacks (no `tsc`, unsupported version, bad path, crash, hang) are asserted as the structural output plus one warning. The CLI test asserts the default output has no `SEMANTIC_BACKEND` diagnostic.

Measured on `7.1.0-beta`, `extract --json` (best of 2 to 3), flag on against off, plus the base commit's output compared byte for byte (identical apart from `durationMs` on all five corpora, with and without `--load-dependencies`). Peak RSS is the larger of the two processes; on Chakra Caliper is 117 MiB and `tsc` 945 MiB.

| Corpus   | Components resolved by the checker |            Off |              On |
| -------- | ---------------------------------: | -------------: | --------------: |
| mantine  |                          332 / 344 |  47 ms, 82 MiB | 588 ms, 287 MiB |
| chakra   |                          598 / 598 |  38 ms, 68 MiB |  9.3 s, 975 MiB |
| shadcn   |                      1,693 / 1,884 |  48 ms, 81 MiB |  1.5 s, 874 MiB |
| antd     |                          206 / 248 |  36 ms, 69 MiB | 508 ms, 443 MiB |
| spectrum |                      1,153 / 1,356 | 71 ms, 118 MiB | 618 ms, 381 MiB |

Chakra's time is `tsc` computing 470,000 shared props' types, not round trips. Before shared props were interned in the merge, Caliper itself held 7.4 GiB on Chakra.

Against the oracle (`scripts/weekly.py --oracle`, flag on, `7.1.0-beta`), wrong verdicts fall to 0.0% on every corpus but antd (0.1%), but the identical-or-spelled share also falls: mantine 91.3% to 22.0%, chakra 100% to 16.4%, shadcn 85.2% to 52.3%, antd 91.9% to 82.3%, spectrum 83.2% to 28.3%. The oracle credits the structural result for keeping a type's name (`StyleProp<MantineSpacing>`); the checker's answer is expanded to its members and compared as "less precise". Keeping the alias name and arguments needs a `PropType` that carries them, which is an output change.

### Supported binaries

Two packages provide the `--api` server, and both are used through the same client; neither is the default checker (the flag stays opt-in).

- **`typescript`** from 7.1 on (`7.1.0-beta`, rc and stable): `node_modules/typescript/bin/tsc`.
- **`tsrs`** (`oxc-project/tsrs`, a Rust port of the TypeScript 7 checker): `node_modules/tsrs/bin/tsrs`, a Node shim that starts the platform binary from `@tsrs/<platform>`. Traced on the wire against `0.23.0-ts7.1.0-beta`: the same method sequence and the same results as `typescript@7.1.0-beta` on three recordings, and identical canonical output on Chakra, rdt-compat and blueprint.

**Discovery.** `--tsc-path` (or `tscPath`) wins and must exist. Otherwise the search walks up from the project root and takes the first `node_modules` that holds either binary; inside one `node_modules`, `typescript/bin/tsc` comes before `tsrs/bin/tsrs`, so an install that has both keeps using `typescript`. Pass `--tsc-path` to choose `tsrs` there. The platform binary (`node_modules/@tsrs/<platform>/tsrs`) can be given with `--tsc-path` too and starts without Node.

**Version rule** (`crates/tsc-client/src/supervisor.rs`, list in `crates/semantic/src/lib.rs`, `SUPPORTED_VERSIONS`). The version is read from the `package.json` of the package that owns the binary: the parent of `bin` for `typescript/bin/tsc` and `tsrs/bin/tsrs`, the binary's own directory for `@tsrs/<platform>/tsrs`. An entry is an exact version or a `major.minor` series. A `tsrs` version reads `<tsrs version>-ts<typescript version>`; it is accepted only when it is listed exactly (today `0.23.0-ts7.1.0-beta`, the build that was traced) and its TypeScript part is also taken by a TypeScript entry (`7.1`). A new `tsrs` release is therefore refused until someone traces it and adds it, as for any other build.

The protocol detail that matters for `tsrs` is unchanged from `typescript`: the client sends `createPrograms[].compilerOptions` (the Node client sends `options`; the server accepts both, and `crates/semantic/tests/replay.rs` pins the key). Set `TSRS_BIN` to a `tsrs` binary to run `crates/semantic/tests/tsrs_real.rs` against the real thing; without it the test returns at once.

## On demand, per component (experimental, off by default)

A long-lived session (the Storybook addon, the lazy `WatchSession`) can attach a warm checker instead of running the whole-project pass above: `semanticTypes: 'on-demand'` (ADR 0022). `caliper-docgen-semantic` keeps one `tsc --api` process on its own thread, answers from a program rooted at only the requested component's file, and returns its props. The program's snapshot is kept between requests and brought up to date with the files the session says changed ("Watch", below). `WatchSession::component` merges them into the structural answer by name (the checker's type wins, a structural prop the checker did not return stays, and `htmlAttributes` applies once after the merge, as in the structural pass) when they arrive within `semanticWaitMs` (default 150 ms); otherwise it returns the structural answer plus an `Info` diagnostic, and the checker's late answer serves the next request of the same component while no source file has changed. Any source-file change, including a file the component only imports, drops every checker answer, so the next request asks again; an answer still in flight at that moment is not kept. The component's `OpaqueType`, `MaxDepthExceeded` and `CircularReference` diagnostics are dropped when the checker's props replace the structural ones, except while a structural prop the checker did not return is still opaque: those diagnostics name the component, not the prop, so they stay until nothing they could be about is left. (The whole-project merge drops unreturned structural props, so it has no such case.)

Measured with `crates/semantic/examples/ondemand_bench.rs` (50 random components, TypeScript 7.1.0-beta): the first answer takes 82 to 137 ms (Chakra 703 ms), a component asked for alone 21 to 41 ms (Chakra 534 ms), and 2 to 9 ms when asked for in batches of 8 to 32. Where the structural pass is behind (shadcn) the checker reports 2.3 times the props; where it is not (Mantine, Chakra, Spectrum) it reports the same count. The in-memory cache, the 64-program process replacement and the failure modes are in the ADR.

Hardening of the on-demand checker (still experimental, off by default):

- **Versions:** `SUPPORTED_VERSIONS` is the `7.1` series (`7.1.0-beta`, rc and stable releases), not one pinned build. A supervisor entry is an exact version or a `major.minor` series; `7.10.0` is not `7.1`. The protocol has no version handshake, so the `package.json` version plus the initialize call that follows the spawn are the check; a build that fails either gives the structural result and one warning. Moving past 7.1 means re-tracing the protocol.
- **Memory:** after every fourth request, once the reply is sent, the worker reads the `tsc` process's resident memory (`/proc`, else `ps`) and replaces the process past `OnDemandConfig::max_rss_mib` (768 MiB; single-component programs peak at 157 to 612). The bound is between requests: one request that needs more finishes first. The 64-program replacement stays: every request counts as a program, whether it built a snapshot or brought the kept one up to date, and a replaced process takes its snapshot with it.
- **Start and stop:** creating the checker and `warm_up` return at once, a missing `tsc` answers the first request with the structural result and one diagnostic, and dropping the checker waits at most `shutdown_wait` (5 s) for a request in progress before letting the worker finish alone.
- **Tests:** the two tests against the real checker (`crates/semantic/tests/on_demand.rs`) run on CI with `CALIPER_TEST_TSC` set to `apps/oracle`'s `typescript-beta`; on CI a missing variable fails them. Locally they skip.
- **Latency, re-measured** (`ondemand_bench`, 30 components, release build, `7.1.0-beta`): shadcn first answer 84 ms, median 21 ms, p90 37 ms; antd first 93 ms, median 36 ms, p90 40 ms. The memory check does not add to a story's wait.

## The algorithm

Mirrors RDT 2.4.0 (`lib/parser.js`). Method names are from the 7.1 client.

1. `getSymbolOfSourceFile(file)`, then `getExportsOfModule`.
2. For each export, `getTypeOfSymbol`, then `getSignaturesOfType(Call)`. A component has a signature whose first parameter is named `props` or is the only parameter. For a class, use `Construct` signatures and read `props` from the instance type.
3. Props type: `getTypeOfSymbolAtLocation(param, param.valueDeclaration)`. Props: the type's apparent properties, plus each member's when it is a union.
4. Per prop, `type` is `typeToString(getBaseConstraintOfType(t) ?? t)` with default flags, so long types truncate as they do in RDT. `required` means the symbol lacks `Optional` and has no code default.
5. `parent` is the first declaration's parent when that is an interface or type alias. `declarations` is every declaration's parent.
6. `description` and `tags` come from `getDocumentationComment` and `getJsDocTags`.

RDT also emits any exported symbol that has a JSDoc comment but no props. We don't. In the fixture corpus, 23 of the 26 entries only RDT emits are of this kind, such as enum members, type aliases and functions.

### The converter (ADR 0026, slice S2)

A prop's checker type becomes a `PropType` from structured answers only (`crates/semantic/src/convert.rs`): the type flags, a union's members (`getTypesOfType`), a reference's arguments, a tuple type's element flags (`getTargetOfType`), an object's signatures, properties and index signatures (one `batchRequests`), and an alias's symbol and arguments. `typeToString` is used for a name to display, and for two facts the API does not state: which parameter is optional and what a tuple element is labeled (`printed.rs` reads them from the printed type and refuses text that does not read to its end; a fact it cannot read is never replaced by a guess).

- **Named boundary.** An interface or class declared under `node_modules` or in the TypeScript lib (`Date`, `Promise<T>`, `HTMLElement`), and an object type a dependency's alias names, stay `named` with their arguments. A function type is always expanded. The lib's utilities (`Partial`, `Pick`, `Omit`, `Record`) are expanded: their result is the type. A dependency's union of literals is expanded, since its members are the values a control offers. React's `ReactNode`, `ElementType`, `CSSProperties` and `ReactElement` are recognized by where they are declared.
- **Caps.** Depth 5, 500 members at the top and 120 below it (a union, an intersection, a tuple's elements), and a budget of 100 properties at the top and 40 below it that counts only the members the project did not declare (inherited from a dependency or the lib, or with no declaration). A cut type is `truncated` with the limit (and, for a width, the total), which `degradedBy` reports. Every descent into the parts of a type (an object, union, intersection, array, tuple or template literal) passes one gate (`Converter::enter`) that checks, before anything is fetched, whether the type encloses itself or is too deep; the width is checked after it, where the type is known. A test has a prop for each cap.
- **Wide object types.** What the project declares is listed whole however wide: an interface of 150 own fields is never cut. Over the budget of members from outside the project, the rest stands as what it is named through, and the project's own members stay beside it. A utility wrapper (`Omit`, `Pick`, `Partial`, `Required`, `Readonly`) is kept as `named` with its arguments when what it applies to has a name of its own: a React DOM attribute type becomes `htmlAttributes` (recognized by the declaring file `/@types/react/` and the interface's name, with the element read from its type argument by the same table the structural path uses, `HTMLAnchorElement` is `a`; an element the table does not know is not guessed), and another dependency or lib type is a `named` layer. The checker keeps no `ComponentPropsWithoutRef`, only the `Omit<..., "ref">` it expands to, which is kept as written. Under the budget the wrapper is expanded (`Pick<ComponentProps<"button">, "disabled" | "type">` has two props). A class or interface of the project that extends such types (`interface BoxProps extends Omit<HTMLAttributes<HTMLDivElement>, "color">`) is an intersection of its own fields and each base type (`getBaseTypes`), a base of the project seen through to its own bases; an instantiation of a generic one, as the type itself or as one base among others, states no bases in terms of its arguments, so the type is cut: its members from outside are `truncated` with their real count beside its own, and the other bases are not named in its place. A type that is not wrapped (`HTMLAttributes<HTMLDivElement>` as a prop) is the `named` it always was.
- **React aliases flattened into a union.** The checker flattens `ReactNode | Fn` into the members of `ReactNode`, so a union that holds all of them (`ReactElement`, `ReactPortal` and an `Iterable` of `ReactNode`, found by declaration, with the primitives) is `reactNode` again beside the rest of its members. `ElementType` below the top of a prop arrives the same way and is wider than the budget: it is recognized (more than 100 of React's own tag names, a fixed table of `JSX.IntrinsicElements`, with `ComponentClass` and `FunctionComponent`) and kept as the terminal instead of cut, and only those tag names go into it: any other string literal the author wrote beside it (`ElementType | "custom-widget"`) stays a member of the union, as does a tag a project adds to `JSX.IntrinsicElements`; narrower, it is listed. `ReactElement`, `JSX.Element` and `ReactChild` stay collapsed to `reactNode` (ADR 0026).
- **Recursion** is a `recursive` marker named by the declaration (the alias or interface) with the instantiation's type arguments. Whether a type is one it is inside of is decided by its type handle, which is the instantiation: `Box<string>` inside `Box<Box<string>>` is another type from the one around it and is expanded, while `Tree<string>` inside `Tree<string>` is a back-reference; an alias of an array or tuple of itself (`type Loop = Loop[]`) ends at the first lap. A handle is only compared inside one conversion, never stored in an answer; type handles change with the order of queries, so a test asks last to first and compares bytes. The per-snapshot memo is keyed by the handle, the depth and the set of enclosing handles, so a hit gives what a miss would.
- **Union order.** The checker sorts a union's members by kind and value. Where the structural pass resolved the same union (in a parsed file, or a loaded dependency), the merge puts the members back in its order, which it read from the type node (`types/semantic/order.rs`). A union the structural pass left opaque keeps the checker's order, with no marker.
- **Debug stat.** With `RUST_LOG=caliper_docgen_semantic=debug`, each component logs its request count and milliseconds, and the session logs requests by method and memo hits.

### The checker's answer wins (ADR 0026, slice S4)

The checker stays an explicit opt-in (`--experimental-semantic-types`, `semanticTypes: 'on-demand'`); the default flips in S8. When it runs, its answer replaces the structural one prop by prop, at each surface:

- **CLI.** `extract --experimental-semantic-types` waits for the checker: every candidate component in one program and one snapshot, as in slice 1. `--semantic-timeout-ms` (default 10,000) bounds the checker's start and each request; past it the run takes the structural result with one `SEMANTIC_BACKEND` warning that says it timed out. It bounds a hang, not the run: on 938 Chakra and MUI wrappers (`~/Projects/caliper-fixtures/gate-large`) the whole answer takes 15 to 17 s with tsrs (its program 71 ms) and finishes at the default bound.
- **Addon and sessions.** Unchanged from ADR 0022: the structural answer first, the checker's within `semanticWaitMs` (default 150), a late answer for the next request. The premise that the checker is too slow for a first story still holds with tsrs: Chakra's `Button` (about 1,100 props) takes a median 562 ms through the Rust client, so it arrives on the next request (see [gate measurements](gate-measurements.md#first-answer-s4)).
- **A prop the checker could not read.** Where the checker's type of a prop holds an `opaque` part (an import or a name it did not resolve, a conditional type it left open, signatures it could not read) and the structural pass read the same prop's type in full, the prop keeps the structural type and its `degradedBy` also lists the checker's reasons: the checker did not confirm the type, so it is marked rather than silently trusted. One `SEMANTIC_BACKEND` `Info` diagnostic counts those props and names their components. A type the checker only cut at a limit (`truncated`) is still its answer, and so is an opaque checker type where the structural one is opaque too. On the 938 wrappers this applies to 60 props, such as Chakra's `cornerShape`, which the checker answers with its error type, and `storageManager`, whose `StorageManager` it left opaque.
- **Gate.** The construct suite's checker lane holds `checker-expectations.json` against `expectations.json`: no row may rank below the structural path unless it lists an `improvement` reason (`apps/oracle/construct-suite/README.md`, "The checker lane").

### Generic components (ADR 0026, slice S5)

A generic component's props parameter has its type parameters open: `function Poly<E extends ElementType = 'div'>(props: PolyProps<E>)` has two props (`as` typed `E`, and its own), where `<Poly />` has every attribute of a `div`. The API has no request that instantiates a signature, so `crates/semantic/src/instantiate.rs` writes the instantiation as source and lets the checker form it:

- **Which instantiation.** The one `<Poly />` renders: the first type parameter's declared default, printed by the checker (`getTypeParametersOfSignature`, `getDefaultFromTypeParameter`, `typeToString`). The checker fills the parameters after it from their own defaults, which may refer to the first. A component whose first type parameter declares no default is unchanged: its props keep the open parameters, since no instantiation is the one a story renders.
- **How.** Once every file is answered, one file, `__caliper_instantiations__.ts` in the project root, declares one line per generic component, `import type { "Poly" as CaliperComponent0 } from "/abs/Poly.tsx"; export type CaliperProbe0 = Parameters<typeof CaliperComponent0<"div">>[0];`, and is added to a second snapshot of the same program through `createSnapshot`'s `fileSystem` layer (`{ "kind": "layer", "files": { path: text } }`), which both tsrs `0.23.0-ts7.1.0-beta` and the Go `7.1.0-beta` read and neither writes to disk. The props are those of `getDeclaredTypeOfSymbol(CaliperProbe0)`, read and converted as any props type is, in that snapshot.
- **When it cannot form.** The file's errors (`getSemanticDiagnostics`) are matched to components by line. A component whose line has an error keeps its structural props, and the fallback reason names the instantiation and the checker's message: `the instantiation Configured<Settings> could not be formed: Cannot find name 'Settings'.` (a default the checker prints by a name only the component's module sees). A props type that is a union after instantiation falls back as any union props type does. No props are read from an instantiation that did not form.
- **Cost.** Nothing for a component that is not generic: whether it is comes with the signature already fetched. A generic one costs three requests to find its default; a run with any costs one more snapshot (4 ms on the fixture, the first snapshot's parsed files reused), three requests for the file (its module, exports and errors) and its release, and one per component for the instantiation's type, then the props requests the component would cost anyway.
- **Gate.** `Poly<'div'>` has at least 260 props and `'a'` differs: 281 and 289 on both checkers (`crates/semantic/src/instantiate.rs`, `crates/semantic/tests/instantiate.rs`, against tsrs in the checker lane and the Go checker in Test / Rust). With the whole-project merge, `@types/react`'s props stay with the inheritance layers as for any component, so the CLI shows the component's own props, `as` as `'div'`.

### Watch (ADR 0026, slice S6)

A warm checker keeps the snapshot it answered from, and the next request brings it up to date instead of building the program again (`crates/semantic/src/on_demand.rs`):

- **Telling the checker.** Every change a `WatchSession` applies reaches its checker before any request that may read it: `update_file` and `ensure_fresh` pass the files they applied, a rebuild or a cold start passes "everything" (`SemanticProvider::changed`, `SourceChange`), in the same order as the requests, through the checker's queue.
- **Bringing the snapshot up to date.** `updateSnapshot` with `fileNotifications` (`changed` for a file on disk, `deleted` for one that is gone) and `ensurePrograms: true`; a request for another file also passes `reconfigurePrograms` with its root. Both checkers need `ensurePrograms`: without it a synthetic program keeps the old text even of a file it was told about, and with it the response lists the files whose text changed. Files not named are kept as parsed. A snapshot nothing changed is answered from as it is; an old one is released once its successor exists.
- **An edit nobody announced.** Neither checker sees an edit it is not told about (an update without the file in `fileNotifications` answers from the old text, and so does one told about another file), so before an answer the kept snapshot is checked: after each reply the files its program read are listed (`getSourceFileNames`, 1,855 for Chakra's `Button`) and stamped by modification time and length, and before the next answer they are stamped again (about 1.5 ms on four threads). A file that changed and was not announced makes the answer come from a fresh snapshot (`createSnapshot` reads everything from the disk), with a `SEMANTIC_BACKEND` warning that names the file. A file modified within 2 s of the snapshot's read cannot be told from a later edit by its stamp, so it is read again at every update until it settles: never a warning, never stale. A file that cannot be listed or stamped makes the next request build afresh.
- **Not seen.** A file created, without an announcement, where an import failed to resolve: it is not in the program, so it has no stamp. The session announces creations; only a writer that bypasses it is missed. A change to the tsconfig's compiler options still needs a new process, as before.
- **Memory.** Every request counts toward the 64 programs of a process, and the memory check is unchanged; a replaced process takes its snapshot with it.

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

**Beta re-probe (2026-10-21, `7.1.0-beta`).** `crates/tsc-client`'s smoke example spawned the beta's `bin/tsc --api`, completed the `initialize` round trip (`caseSensitivity`, `currentDirectory`) and shut down cleanly with exit status 0, so framing and shutdown behave as the fake-tsc harness assumes. `TSC_PACKAGE_DIR=<dir> cargo run -p caliper-docgen-tsc-client --example smoke_real_tsc` repeats it against any installed `typescript`. The beta's `proto.generated.d.ts` still exposes every checker call the algorithm uses: `getTypeAtLocation`, `getTypeOfSymbol`, `getDeclaredTypeOfSymbol`, `getSignaturesOfType`, `getResolvedSignature`, `getReturnTypeOfSignature`, `getBaseTypes` and `getPropertiesOfType`. The fixture-corpus parity run below was re-run on the beta on 2026-10-23 (`apps/oracle` imports `typescript-beta`, `npm:typescript@7.1.0-beta`, since the `unstable/*` entry points are gone). Two beta behaviors to know: the server panics (`interface conversion: interface {} is string, not float64`) when `createProgram` gets string-valued enum options such as `target: 'es2022'`, so pass options from `parseConfigFile`; and `true`/`false` literal types carry neither `value` nor `intrinsicName`, so read them with `typeToString`. Discovery over a whole project ran in the oracle: 33 to 1,531 components per corpus.

**Churn.** About 20 commits touched the protocol definition (`proto.go`) between 2026-08-20 and 2026-09-18. `updateSnapshot`, project IDs and `createProgram` params were reshaped on 09-17 and 09-18. Every call the algorithm uses ran unchanged on the 09-20 and 09-26 nightlies. Re-verify on the beta.

**Freshness.** tsc sees an edit to a dependency only when told:

```js
const s1 = await s0.update({ fileNotifications: { changed: [file] }, ensurePrograms: true })
```

With the notification, the dependent component's props updated (0.5 ms update, 2.8 ms query). An unnotified disk edit stayed stale (2026-09-20). Both incremental paths now notify it (`WatchSession::update_file` and `WatchSession::ensure_fresh`), and an edit neither saw is caught before an answer: see "Watch" above.

## Evidence

Method: RDT 2.4.0 on typescript@6.0.3 (`apps/oracle`'s RDT is pinned to a TS6 API — see "The RDT oracle" below) with the harness options (`shouldExtractLiteralValuesFromEnum`, `shouldRemoveUndefinedFromOptional`, `propFilter: { skipPropsWithoutDoc: false }`), against the algorithm above on the nightly named in each row, or `7.1.0-beta`. The harness that ran it is not in the repository: the table is the record.

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

`apps/oracle` depends on TypeScript 7.1 directly (for the oracle), but RDT 2.4.0 crashes on import against it (`ts.JsxEmit` is undefined). RDT now resolves its `typescript` peer to `typescript@6.0.3` instead, via the aliased `typescript-beta` dependency (`npm:typescript@7.1.0-beta`, pinned exactly) the oracle imports from — no pnpm override needed, since pnpm overrides don't reliably rewrite a peer dependency's resolution (tried `pnpm-workspace.yaml`'s `overrides: { "react-docgen-typescript>typescript": ... }`; the lockfile recorded it but RDT kept resolving against whatever `typescript` apps/oracle depended on directly).

The removed harness's `compare.ts`/`analyze.ts` (caliper-docgen vs RDT/react-docgen) compared only name, `required`, type and default, and rendered our types themselves in `run-ours.ts` (objects print as `{ ... }`), so their type mismatches mixed resolver and renderer differences. Those limits went with it: `scripts/weekly.py` compares Caliper's own and `@types/react` props with RDT's by name, and the oracle compares prop types structurally against the TS 7.1 checker (`docs/measurement.md`).

## Open decisions

1. **Output type.** Checker types don't fit `PropType` losslessly: `Named` has no file or symbol, and `Tuple` has no optional or rest elements (schema version 5 adds them, with labels and the other kinds ADR 0026's slice S1 lists). Add a variant or map into existing ones. `ExtractionOutput` carries `SCHEMA_VERSION`; a new variant bumps it.
2. **`ref` and `key`.** RDT emits both, and nothing else records forwardRef metadata. Semantic mode must match whatever the structural path does.
3. **Trigger.** Run on every exported component or only on files with degrade diagnostics. Bare generic parameters and TypeScript utility types resolve to `Named` with no diagnostic by design, so the second misses them.
4. **Union order.** Sort in canonical output, or keep source order for structural results.

## Not verified

- Cost through the Rust client. Every measurement uses the Node client.
- Overloads with more than one signature, static sub-components (`Menu.Item`), and barrel re-exports, where RDT names components by the barrel file.
- `@default` and `@type` JSDoc overrides.
- Projects larger than the 44-file corpus.
- What the `MappedType` accessors added on 2026-09-23 ([#64397](https://github.com/microsoft/TypeScript/pull/64397)) expose beyond resolved properties.
