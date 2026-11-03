# Measurement

Two things, both small: a weekly record of how Caliper compares with react-docgen-typescript (RDT) and react-docgen on five public repos, and `just bench` for a change that touches the resolver. Nothing here blocks a pull request.

## Weekly record

`.github/workflows/weekly.yml` runs on Mondays and by hand. `scripts/corpora.py` fetches each repo at a pinned commit and installs its dependencies with its own package manager and lockfile (lifecycle scripts off, caches under `.corpora/`, gitignored), then removes tests, stories, demos and `.d.ts` files from the source directory so every tool reads the same files. `scripts/weekly.py` runs the tools.

| Corpus | Repository | Commit | Source directory | Install |
| --- | --- | --- | --- | --- |
| mantine | `mantinedev/mantine` | `e47e45af06427b3b2497100c2b849935c23af30d` | `packages/@mantine/core/src` | yarn, immutable |
| chakra | `chakra-ui/chakra-ui` | `fa9696f328ea5a0df8685c6d8460edbd779b63b9` | `packages/react/src` | pnpm, frozen |
| shadcn | `shadcn-ui/ui` | `ba1503c64472b82256d9de95e2e62c5901a38e8b` | `apps/v4` | pnpm, frozen |
| antd | `ant-design/ant-design` | `ba7ccbaa7b4b58ac1ff8e6fe2e78dfa10bfb5538` | `components` | npm, no lockfile (the repo commits none) |
| spectrum | `adobe/react-spectrum` | `797762ed3526ea12f089e321bd3891066affad70` | `packages` | yarn, immutable |

It records, per corpus and tool, the wall time and peak RSS of one whole run (process start and Node start included), and the size of Caliper's canonical JSON. Caliper runs with `--load-dependencies`; its time is the median of five runs. RDT and react-docgen run once each.

Each run's peak RSS is that process's own. `scripts/corpora.py` starts every measured command through a small helper that forks it and reads its `wait4` usage. Reading it straight from the weekly script gave one number for every run after the largest: on Linux a child's `ru_maxrss` starts at the highest RSS its parent ever had, and the script's own peak (it holds RDT's output for a large corpus) became every later tool's "peak". The first weekly record shows 1,593 MiB for all three tools on shadcn, antd and spectrum, which is that. `scripts/test_weekly.py` checks that a large parent no longer leaks in.

### The reference

Recall is measured against RDT's output, cleaned in two steps. Both are by what RDT reports, never by library or name lists, and `weekly.json` keeps their counts (`reference` per corpus) so a change in either is visible.

| Step | Rule |
| --- | --- |
| Not components, left out | More than half of the props are declared in TypeScript's own `lib.*.d.ts` (the parameter is a `string`, a `Request`, a function: `GET`, `loadHighlighter`, `convertLegacyProps`). The name is a hook's (`use` and a capital). The only props are a `value` (and `children`) declared by `@types/react`, which is a context's Provider (`FormContext`). The name starts lowercase, is not the file's or directory's name (RDT names a default export after those) and no prop comes from `@types/react`, so it is no JSX tag and takes no tag's attributes (`genLinkStyle`, `generateMetadata`). Every prop is a `Promise<...>`, which a router hands over (`page`). |
| Repeats, counted once | RDT lists one component under each file that re-exports it, and names a default export after the file or directory (antd's `col/index.tsx` default-exports `Col` from `../grid` and RDT reports `col` there; Caliper reports the defining file). An entry that Caliper does not report in its own file pairs by evidence: where the default export, or the named export, of RDT's file is defined (followed through `export default X`, `import`, `export { X } from` and `export { default as X }`), or else the file that declares its props when Caliper reports a component of that name there. The same evidence works inside one file: a default export that is another name (`const InternalButton = ...; export default InternalButton`, which RDT calls `button`), or the component whose `displayName` RDT reports. Both are guarded: the RDT name must be the file's or directory's own, so a member such as `Form.List` is never paired with `Form`, and a Caliper key that another RDT entry matches exactly is never a target. A call around the name (`memo(Foo)`, `React.forwardRef(Foo)`, `Object.assign(Foo, { Item })`), a type assertion (`Foo as T`, `Foo satisfies T`, `Foo!`) and a chain of local aliases (`const A = memo(B)`) lead to the declaration; Caliper's `displayName` for it counts as its name. Entries that pair with the same Caliper component are one reference item. |

The record shows, per corpus: RDT's entries with props, how many are not components, how many repeats were merged, the reference size, how many match in the same file, how many are paired (by default export, default alias, `displayName`, named export or declaration) and how many stay unpaired. `weekly.json` also records the `reference_version`: the rules above are version 2 (a record without it is version 1), and recall is not compared with a record made under other rules, so a change to them cannot read as a regression or hide one. Pairing needs the names to agree: the component the export resolves to must have the name Caliper reports, so an entry never pairs with an unrelated component just because it is the only one in its file. When several RDT entries pair to one component, the reference takes the props of the entry in the component's own file, else the union of the repeats'. An unpaired item is a component Caliper does not report where RDT does, and it stays in the denominator.

The 2026-10-31 effect on the pinned corpora, Caliper's component recall (own and `@types/react` props are unchanged by the exclusions and move only a little with the pairing):

| Corpus | Reference before |                                           After | Components before | After |
| ------ | ---------------: | ----------------------------------------------: | ----------------: | ----: |
| shadcn |             1594 |                   1573 (21 left out, 0 repeats) |             95.2% | 96.5% |
| antd   |              258 | 203 (43 left out, 12 repeats merged, 12 paired) |             64.0% | 81.3% |

Exclusions alone give 76.7% on antd, pairing alone 68.3%.

The 2026-11-02 pairing (same-file default aliases, `displayName`, wrappers, assertions, alias chains), replayed on the pinned commits with RDT's output and Caliper's at that day's `main`:

| Corpus | Reference before | After | Components before |        After |
| ------ | ---------------: | ----: | ----------------: | -----------: |
| shadcn |             1573 |  1573 |      1518 (96.5%) | 1518 (96.5%) |
| antd   |              207 |   202 |       165 (79.7%) |  186 (92.1%) |

Each corpus entry's `reference.unmatched` says why the items Caliper reports nowhere are missing: `by_cause` counts them and `listed` names the first 15 of each cause as `file: name`. The cause is what the item's file shows, so it says where to look, not what is wrong:

| Cause | The file shows |
| --- | --- |
| `compound_member` | An assignment to a member of another component (`Form.List = List`). |
| `object_member` | The name as a function-valued key of an object literal (`logo: () => ...`). |
| `dependency` | An alias of something a package exports (`const Tooltip = Recharts.Tooltip`), or an export that ends in a package. |
| `call_result` | A value a call builds (`createIcon(...)`, `generateCalendar(...)`). |
| `file_with_other_components`, `file_without_components` | A declaration Caliper does not report, in a file where it reports others, or none. |

The 16 antd items still unpaired are not pairing cases: the date picker and calendar families (built by factory functions), `Parent.Member` entries Caliper reports under the parent (`Form.List`, `Mentions.Option`, `TreeSelect.TreeNode`), and helpers RDT lists as components (`confirm`, `dropIndicator`). shadcn has no re-export of this kind in the pinned commit, so nothing pairs there; its unpaired items are `Icons.logo`-style object members, aliases such as `const ComboboxRoot = Primitive.Root` and factory results, which Caliper does not detect.

Recall is measured against that reference, on the components both tools report (same file, same name, or paired):

| Measure              | Meaning                                                                                   |
| -------------------- | ----------------------------------------------------------------------------------------- |
| Components           | The reference's components that the tool also reports                                     |
| Own props            | Of those components, RDT's props not declared by `@types/react` that the tool lists       |
| `@types/react` props | RDT's props that `@types/react` declares (`onClick`, `aria-*`, `ref`) that the tool lists |
| ... with layers      | The same two, also counting a prop held in a collapsed inheritance layer                  |

By default (`collapseInheritedProps: dependencies`) Caliper collapses a large ambient inherited dependency source (DOM and ARIA attributes, style systems; more than 30 props) into a layer (ADR 0012) and lists only its notable members as props. A dependency's own API is listed in full (a source past 200 props that is not ambient by name still collapses). The Storybook addon runs with `collapseInheritedProps: none`, which lists every inherited prop; the layers below are what the CLI, the NAPI binding and agents reading the output see. A prop in a layer is in the output, named by the layer's `members`, but is not in the flat list, so the table reports both: the plain columns say what the default output lists, "with layers" says what it holds. react-docgen writes no layers, so its two columns are equal.

react-docgen parses each file with its default resolver and importer, as Storybook does. The importer follows relative and package imports to the file that declares a type, so a props interface imported from a sibling file is read. It does no type inference: it lists none of React's inherited attributes, and a generic, mapped or conditional type stays the source text. That is the point of the comparison. A file it throws on is counted (`failed`) and listed (`failedFiles`), not dropped silently.

The run uploads `weekly.json` and a table to the job summary. It compares with the last successful run's `weekly.json` and opens, or comments on, one issue labelled `weekly-measurement` when Caliper's time or memory is more than 10% worse, or its recall is more than 2 points lower, on any corpus. There is no committed baseline: the previous run is the baseline.

### TypeScript's checker as the oracle

RDT has mistakes of its own. With `--oracle` the run also asks TypeScript 7.1's checker (`apps/oracle/src/oracle.ts`, `typescript/sync` of `7.1.0-beta`) for each prop's type and compares it structurally with Caliper's canonical `--json`. The weekly table shows the share of props that agree (identical, or the same type spelled by name) and the share the checker contradicts. This is information only: the checker is a beta and the backend that uses it is an opt-in (`docs/type-checker-integration.md`).

| Verdict           | Meaning                                                                          |
| ----------------- | -------------------------------------------------------------------------------- |
| Identical         | Both sides describe the same type                                                |
| Spelled           | The same type, Caliper by name where the checker expands it                      |
| Less precise      | Caliper's type is looser or left unevaluated                                     |
| Wrong             | Caliper's type describes something else, such as a literal union missing members |
| Checker any/error | The checker has no type                                                          |

#### All three tools against the checker

RDT and react-docgen are held to the checker too, by the same rules, so the comparison does not take either as the truth. `oracle.ts` also takes the dumps `scripts/rdt.mjs` and `scripts/rdg.mjs` write (`--rdt`, `--rdg`). Both tools state a type as source text, which `apps/oracle/src/tool-norm.ts` reads with TypeScript's parser into the shape the comparison uses; a type a tool printed in part (`Foo<...>`, `{string}`) or did not evaluate is unreadable, which scores as less precise and never as wrong. The scoring is `apps/oracle/src/tool-score.ts`.

The **universe** is every own prop the checker finds on an exported function whose only parameter, or `props` parameter, is the props type. React's inherited DOM attributes, `ref` and `key` are not in it. It includes some functions that are not components, because the checker has no notion of one, so read the shares as a comparison between tools, not as a share of real components. For each tool the record has, in `oracle.tools`:

- **Agreement of the whole universe** (`agree_universe`): identical or spelled, over every prop in the universe. A prop the tool omits counts against it. This is the headline.
- **Agreement of listed props** (`agree_listed`) and the share the checker contradicts (`wrong_listed`): over the props the tool lists, which is what Caliper's own `agree` and `wrong` have always been.
- **Omitted props**, by where the checker declares them (`missing_by_stratum`): the component's own file, another file of the corpus, or a dependency. A prop Caliper lists only as a member of a collapsed inheritance layer is counted apart (`collapsed`), because that is a choice and not an omission.
- **Why a prop is omitted** (`missing_by_reason`): the tool reports no such component, it reports the component but not the prop, or it threw on the file.
- **react-docgen's thrown files** (`failed_files`): files its default resolver throws on, 17% to 21% of the files on antd and shadcn. Their props are omitted, so they cost react-docgen agreement; they have their own column so the cost is visible.

`oracle.shared_props`, `agree` and `wrong` keep their meaning, so the weekly history does not move. A local run on antd and shadcn at the commit that added this reads, for the whole universe: Caliper 39.7% and 40.1% (its `collapsed` layers and any component it does not report count against it), RDT 44.8% and 84.8%, react-docgen 24.6% and 1.9%. The step is information, never a gate, and the rules are validated on antd and shadcn only: Mantine, Chakra and Spectrum need a weekly run.

## `just bench`

```bash
just bench                 # this checkout, uncommitted work included, against origin/main
just bench my-branch       # a branch
```

`scripts/bench.py` builds both in release mode, then times `extract --json` over Mantine and Chakra (source directory only, nothing installed, dependency loading off, no cache) with the two builds interleaved and alternating which goes first. It prints the median wall time and peak RSS of each and their ratio. The first run fetches the two source directories into `.corpora/`.

With nothing changed, the ratio stays inside 1.04 for time (95th percentile, 100 trials per corpus on CI and a laptop) and inside 1.12 for memory on CI runners. More than 1.10× in time or 1.20× in memory is worth looking at. Nothing enforces that. Run it before a resolver change goes up for review, and say in the pull request when a cost is intended.

## What this replaces

Before the weekly record, four workflows (Equivalence, Parity, Scorecard and Allow-list empty) ran against every pull request or `main` push, backed by `scripts/equivalence.py`, a per-pull-request allow-list directory and two committed baselines. They caught one regression in speed and one in output that review would have missed. They also failed on runner noise, on bases that had moved and on their own allow-list, and a lot of one week of merge work went to keeping them green. Git history has them.

What we give up: a pull request that slows the resolver or changes output no longer fails by itself. Review, `just bench` and the weekly issue catch it, the last one days later.

## Results, 2026-10-23

Measured with the parity script this replaces: `main` at `7bc120f4`, Apple Silicon laptop, 7 runs, with `--load-dependencies`.

| Corpus   | Components matched | Non-React props reported |           Caliper |               RDT |
| -------- | -----------------: | -----------------------: | ----------------: | ----------------: |
| mantine  |            331/389 |                    99.7% |    55 ms, 109 MiB |    1.3 s, 458 MiB |
| chakra   |            592/773 |                    98.6% | 550 ms, 1,461 MiB | 24.8 s, 1,550 MiB |
| shadcn   |          1519/1594 |                    39.5% |   115 ms, 181 MiB |  4.0 s, 1,149 MiB |
| antd     |            162/258 |                    80.3% |    86 ms, 127 MiB |  2.4 s, 1,033 MiB |
| spectrum |          1087/1408 |                    99.7% |   130 ms, 219 MiB |    2.2 s, 828 MiB |

RDT's memory is its process's peak RSS and its time excludes Node's start; Caliper's time includes process start.

## Collapse by relevance, 2026-11-02

A dependency source used to collapse into a layer of names once it declared more than 30 props. It now collapses when it is ambient (DOM and ARIA attribute families, style systems; clauses in `crates/core/src/resolver/ambient.rs`) and past 30 props, or when it is not ambient but past 200 props (`LARGE_API_SOURCE`). Every other dependency source is listed with its types. The rule is checked against the oracle's classifier (`apps/oracle/src/layer-only.ts`) by a fixture of its verdicts (`crates/core/src/resolver/ambient_parity.json`).

**Who it changes.** The Storybook addon runs with `collapseInheritedProps: none` (`packages/storybook/src/provider.ts`), which lists every inherited prop, so the rule does not change what the addon shows. The output is byte-identical between the two builds on all five corpora in that mode. The rule changes the CLI and NAPI default, and so what agents and TOON readers see.

`scripts/collapse_relevance.py` ran it: two builds (`10f1f072`, the base, and this branch), five corpora on the pinned commits, five interleaved runs, `--no-cache`, whole-process wall time and peak RSS from `/usr/bin/time -l`, load average 29 at the start, so read differences under about 20% as noise: the `none` runs, which execute the same code and print identical output in both builds, differ by up to 16% in a lazy session.

Default mode, base to this branch (canonical JSON, `--html-attributes full --include-unexported --load-dependencies`):

| Corpus   | Bytes                          | Collapsed layers | Wall time      | Peak RSS       |
| -------- | ------------------------------ | ---------------- | -------------- | -------------- |
| antd     | 3,954,611 to 4,143,776 (+4.8%) | 24 to 0          | 0.10 to 0.10 s | 119 to 118 MiB |
| shadcn   | 8,629,920 to 8,781,666 (+1.8%) | 112 to 106       | 0.29 to 0.31 s | 317 to 313 MiB |
| chakra   | 8,298,481 to 8,357,008 (+0.7%) | 2 to 0           | 0.50 to 0.42 s | 382 to 385 MiB |
| mantine  | 6,026,955, identical           | 0 to 0           | 0.15 to 0.15 s | 153 to 150 MiB |
| spectrum | 6,941,080, identical           | 0 to 0           | 0.16 to 0.15 s | 183 to 183 MiB |

antd's 24 layers were all API (`@rc-component` dialog, select, input-number and menu props), as were shadcn's six (react-day-picker's `PropsBase`, Base UI's `ComboboxRootProps`) and Chakra's two (zag-js `ComboboxProps`, `DatePickerProps`); shadcn's 106 `GlobalDOMEvents` layers (64 handlers each) stay collapsed. Mantine and Spectrum have no dependency layers: their sources are project sources.

Type correctness, the oracle's verdicts over the props both sides list (default mode, base to this branch): antd 3,075 to 3,426 shared props, identical 1,772 to 1,972, spelled 1,096 to 1,229, less precise 206 to 224, wrong 0 to 0; shadcn 8,353 to 8,641 shared, wrong 8 to 8; Chakra 486,752 to 486,832, wrong 1 to 1; Mantine and Spectrum identical. The props that become listed are judged as correct as the ones already listed (antd agree 93.1% to 93.4%, shadcn 90.7% to 90.6%), and no wrong verdict appears.

**The Storybook configuration (`none`), which the addon pays today.** Identical output between the builds; per corpus, the new build:

| Corpus   | Bytes     | vs the base default | Wall time | Peak RSS | Lazy: first component, first 50 |
| -------- | --------- | ------------------- | --------- | -------- | ------------------------------- |
| antd     | 4,143,776 | +4.8%               | 0.10 s    | 119 MiB  | 58 ms, 88 ms                    |
| shadcn   | 9,466,482 | +9.7%               | 0.32 s    | 312 MiB  | 134 ms, 388 ms                  |
| chakra   | 8,357,008 | +0.7%               | 0.37 s    | 384 MiB  | 181 ms, 618 ms                  |
| mantine  | 6,026,955 | 0                   | 0.14 s    | 154 MiB  | 69 ms, 170 ms                   |
| spectrum | 6,941,080 | 0                   | 0.14 s    | 184 MiB  | 111 ms, 134 ms                  |

Listing every inherited prop costs bytes only where a dependency layer exists (shadcn's `GlobalDOMEvents`, 106 layers of 64 handlers); time, memory and the lazy first-component and 50-component times are within noise of the default mode on every corpus. Lazy sessions in default mode, base to this branch (first component, first 50): antd 60 to 63 ms and 93 to 94 ms, shadcn 118 to 157 ms and 283 to 350 ms (the same build running `none` moves 335 to 388 ms: noise), Chakra 166 to 166 ms and 568 to 553 ms, Mantine 73 to 83 ms and 183 to 199 ms, Spectrum 118 to 121 ms and 155 to 149 ms.

**The case the five repositories cannot show: an app that consumes a style system from `node_modules`.** The corpora are design-system repositories, where Chakra's and Mantine's style systems are project sources and never collapse under the default. A scratch app with four components each on `@chakra-ui/react`, `@mantine/core` and `@mui/material` (pinned in `package.json`, installed with `npm`, source in a directory beside `node_modules`) shows it:

- **Chakra v3:** the style system is `SystemProperties`, 705 props, under a name none of the clauses knows. Without a size valve the new rule listed it: 516,158 to 1,955,639 bytes (3.8 times), 0.04 to 0.08 s, 74 to 92 MiB, for four components. That is the blow-up (ADR 0010: Chakra 560 to 398,910 props) the size rule existed for. With the valve (a non-ambient dependency source past 200 props collapses) the output is the base's, byte for byte. The largest source that is an API in any corpus or app is 75 props (react-day-picker's `PropsBase`); 200 is 2.7 times that.
- **Mantine:** `MantineStyleProps` (50 props) matches the `*StyleProps` clause and collapses as before.
- **MUI:** `BaseTextFieldProps` (31 props) is API and is now listed: 359,062 to 375,089 bytes (+4.5%).
- In the Storybook configuration the Chakra app is 1,955,639 bytes, 0.07 s and 92 MiB for four components, against 516,158 bytes, 0.04 s and 74 MiB in the default.

**Recommendations, with the numbers behind them.**

- Keep the valve at 200. It is the smallest the data justifies: one measured style system (705) is far above it and no API reached a third of it. Revisit it if a corpus or app shows an API source between 75 and 200 props with a cost problem, or a large non-ambient source that is an API.
- A larger consumer app on Chakra was not measured (four components). The output interns prop sets, so the cost per component is probably sub-linear, but only the four-component figure is a measurement.
- The addon's `none` costs +9.7% bytes on shadcn and 3.8 times in a Chakra consumer app, with no measurable time or memory cost on the five corpora; the per-component cost of `none` in a lazy Chakra-consumer session is the number to take if the addon's default is ever reconsidered.
