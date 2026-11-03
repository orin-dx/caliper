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

Caliper collapses an inherited dependency source of more than 30 props into a layer (ADR 0012) and lists only its notable members as props, as Storybook does. A prop in a layer is in the output, named by the layer's `members`, but is not in the flat list, so the table reports both: the plain columns say what the default output lists, "with layers" says what it holds. react-docgen writes no layers, so its two columns are equal.

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

## `just bench`

```bash
just bench                 # this checkout, uncommitted work included, against origin/main
just bench my-branch       # a branch
```

`scripts/bench.py` builds both in release mode, then times `extract --json` over Mantine and Chakra (source directory only, nothing installed, dependency loading off, no cache) with the two builds interleaved and alternating which goes first. It prints the median wall time and peak RSS of each and their ratio. The first run fetches the two source directories into `.corpora/`.

With nothing changed, the ratio stays inside 1.04 for time (95th percentile, 100 trials per corpus on CI and a laptop) and inside 1.12 for memory on CI runners. More than 1.10× in time or 1.20× in memory is worth looking at. Nothing enforces that. Run it before a resolver change goes up for review, and say in the pull request when a cost is intended.

## Checker modes

`scripts/checker_modes.py` measures the experimental TypeScript 7.1 checker modes against the structural default on the corpora `scripts/corpora.py` sets up. It is run by hand for now and is not part of the weekly record.

```bash
cargo build --release -p caliper-docgen && pnpm --filter caliper-docgen-napi build:napi
python3 scripts/checker_modes.py --bin target/release/caliper-docgen --runs 3 --count 50 --fallbacks \
  --out report.json --diff-out diff.json --pairs-out pairs.json
```

What it runs, per corpus: the structural default (`extract --format rdt` with dependency loading), the same with `--experimental-semantic-types` (the CLI's whole-project checker pass), and one lazy session driven the way the Storybook addon drives it (`scripts/checker_modes_session.mjs`: `createSession`, `initializeSessionIndex`, then `sessionComponentAnswers` per component) with the checker off and with `semanticTypes: 'on-demand'` at a 150 ms and a 600 ms wait. `--fallbacks` also runs the CLI and a session against a missing `tsc` and one that exits at once.

What it reports:

- Wall time and peak RSS, median with min and max over `--runs` runs, and the machine's load average before and after. Peak RSS is the largest single process (`corpora.run_once`), so a checker run reports the larger of Caliper and `tsc`; the session script also reports `tsc`'s own peak.
- First load for a session: start (create plus index), the first component, and the first 10 and 50 components, one request at a time, in a seeded order that is the same in every mode.
- For the on-demand mode: the share of first requests the checker answered within the wait, how many timed out, how many kept the structural result for the checker's own reasons (not exported, a class component, generic props), and the share answered once the late answers had arrived.
- The props that differ from the structural result: type, required, default and presence. `@types/react` props are counted apart (the whole-project checker pass leaves them out), and a type that differs only in spelling (spacing, `| undefined`, union order, `Array<T>`) is counted as cosmetic. A rest or optional parameter marker is a difference. `--diff-out` has one row per differing prop (without those two kinds, unless `--diff-all`), `--pairs-out` one entry per distinct pair of structural and checker facts with a count, a category and an example, most common first.
- The checker oracle's verdict on each mode, for information only. The oracle is the same checker, so it agrees with a checker mode almost by construction, and it credits the structural result for keeping a type's alias name while it scores the checker's expanded union as less precise (`docs/type-checker-integration.md`). Do not read that agree figure as accuracy; judging the checker modes needs a reference that is not the checker.

### Results, 2026-11-02

Measured by hand on `main` at `c267c777`, on a laptop with a load average of 5 to 16 (other jobs were running), on local checkouts of the five repos that are not at the commits `scripts/corpora.py` pins, so absolute numbers and the Spectrum oracle figure do not compare with the weekly record. Use the shapes, not the digits; re-run on the pinned corpora before deciding anything. The time and memory figures use `--format rdt`, which does not intern props, so they are larger than the weekly record's `--json` figures.

| Corpus | Structural | Whole-project checker | First 50, structural | First 50, on-demand (150 ms) | Checker answered in time |
| --- | --- | --- | --- | --- | --- |
| mantine | 208 ms | 1.5 s | 108 ms | 2.2 s | 96% |
| chakra | 1.7 s | 12.7 s | 387 ms | 7.8 s | 0% (56% at 600 ms) |
| shadcn | 763 ms | 1.7 s | 80 ms | 1.6 s | 92% |
| antd | 79 ms | 573 ms | 30 ms | 1.4 s | 77% |
| spectrum | 165 ms | 768 ms | 31 ms | 1.7 s | 96% |

- **First load, checker off:** session start 39 to 77 ms, the first component 0 to 63 ms, 10 components 8 to 124 ms and 50 components 30 to 387 ms (Chakra is the high end), the process peaking at 173 to 366 MiB with Node and the binding. That is the same order as `docs/performance.md` "Lazy sessions" (2026-10-24), measured there on the Rust example and on the pinned corpora, so not a like-for-like check.
- **On-demand checker:** the first answer takes 44 to 64 ms (218 ms on Chakra, where it timed out), and a request the checker has not answered waits the whole 150 ms before it takes the structural answer. 50 sequential requests cost 1.4 to 2.2 s against 30 to 108 ms without it, which is about 30 to 45 ms per component. Chakra's checker takes over 500 ms per component, so at the default 150 ms wait it answered none of the first requests and half of them on the second pass; at 600 ms the first answer took 681 ms and 50 components 26 s. `tsc` peaked at 197 to 300 MiB.
- **The fallbacks work:** a missing `tsc` and a `tsc` that exits at once each give exit code 0, output byte-identical to the structural result, and one `TypeScript checker backend unavailable` warning, in the CLI and in a session. A hanging or slow `tsc` is covered by the tests in `crates/semantic`.
- **What differs, whole-project checker, props that are not `@types/react` and not spelling only:** about 500,000 are an alias name written out (Chakra's `ConditionalValue<SpacingToken | ...>` becomes hundreds of literals, Mantine's `StyleProp<MantineSpacing>` becomes its union), 9,772 are props only the checker lists, 5,744 other type text, 2,452 an alias name where structural wrote the type out, 1,210 props only structural lists, and 65 are a rest or optional parameter marker the checker's text does not carry (`(...args: any[]) => void` becomes `(args: any[]) => void`, `(percent?: number) => ReactNode` becomes `(percent: undefined | number) => ReactNode`). Required and default never differ.
- **Props only the checker lists** are mostly DOM event handlers (`onAuxClick`, `onGotPointerCapture`) on shadcn and Spectrum components that wrap a Radix or React Aria primitive, where the structural pass reaches fewer props. They are also Mantine's `__size`, `__vars` and `__clearable`, which the checker names `___size`, `___vars` and `___clearable`: TypeScript's internal escaping adds an underscore and the checker mode does not take it off.

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
