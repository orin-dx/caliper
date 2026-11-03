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

#### All three tools against the checker

RDT and react-docgen are held to the checker too, by the same rules, so the comparison does not take either as the truth. `oracle.ts` also takes the dumps `scripts/rdt.mjs` and `scripts/rdg.mjs` write (`--rdt`, `--rdg`). Both tools state a type as source text, which `apps/oracle/src/tool-norm.ts` reads with TypeScript's parser into the shape the comparison uses; a type a tool printed in part (`Foo<...>`, `{string}`) or did not evaluate is unreadable, which scores as less precise and never as wrong. The scoring is `apps/oracle/src/tool-score.ts`.

The **universe** is every own prop the checker finds on an exported function whose only parameter, or `props` parameter, is the props type. React's inherited DOM attributes, `ref` and `key` are not in it. It includes some functions that are not components, because the checker has no notion of one, so read the shares as a comparison between tools, not as a share of real components. For each tool the record has, in `oracle.tools`:

- **Agreement of the whole universe** (`agree_universe`): identical or spelled, over every prop in the universe. A prop the tool omits counts against it. This is the headline.
- **Agreement of listed props** (`agree_listed`) and the share the checker contradicts (`wrong_listed`): over the props the tool lists, which is what Caliper's own `agree` and `wrong` have always been.
- **Omitted props**, by where the checker declares them (`missing_by_stratum`): the component's own file, another file of the corpus, or a dependency. A prop Caliper lists only as a member of a collapsed inheritance layer is counted apart (`collapsed`), because that is a choice and not an omission.
- **Why a prop is omitted** (`missing_by_reason`): the tool reports no such component, it reports the component but not the prop, or it threw on the file.
- **react-docgen's thrown files** (`failed_files`): files its default resolver throws on, 17% to 21% of the files on antd and shadcn. Their props are omitted, so they cost react-docgen agreement; they have their own column so the cost is visible.

`oracle.shared_props`, `agree` and `wrong` keep their meaning, so the weekly history does not move. A local run on antd and shadcn at the commit that added this reads, for the whole universe: Caliper 39.7% and 40.1% (its `collapsed` layers and any component it does not report count against it), RDT 44.8% and 84.8%, react-docgen 24.6% and 1.9%. The step is information, never a gate, and the rules are validated on antd and shadcn only: Mantine, Chakra and Spectrum need a weekly run.

### An independent reference

The oracle above scores a tool against the TypeScript checker alone. That cannot judge a mode that uses the checker, which would agree with itself, and it cannot say which side is right when they differ. `scripts/reference_sample.py` (`apps/oracle/src/reference.ts`) judges a sample of props by two methods that fail differently:

- **A, the checker's type.** The same normalized type the oracle compares (`oracle-checker.ts`).
- **B, the declared type.** What the prop's declaration writes, read from source text with TypeScript's parser (`reference-declared.ts`), never from the checker's type printer. The checker is asked only where the declaration is. B abstains where the declaration leaves the type to be computed (an indexed access, `keyof`, a conditional type), writes no type (an unannotated member, a method with no return type, or a value such as `count: 1` in an object literal that the checker infers the type of), has no declaration (a mapped type's synthesized member), or has several that differ.

They are compared with the oracle's own rules (`oracle-compare.ts`), B standing in for the tool. Identical or spelled is agreement. B looser than A (a name or a type parameter A evaluated) is B saying less, so A stands alone. B contradicting A is **contested**: it is listed in the review file for a person, never counted for or against a tool. Where A has no type (`any`, an error) and B does, B stands alone.

A tool's prop is then **correct** (agrees with the reference), **less precise**, **wrong**, **contested**, **no reference**, **missing**, or **layer**. A component lists a prop when it is in its `props` or its `notableInherited` (the curated props of a collapsed source, which a props table lists after the component's own; the component's own wins a clash). Where the declaration stands alone and only names a type (the checker could not resolve it), a tool that gives the same name is correct, and one that gives another type is at most less precise, because the name cannot be seen into. A prop the tool does not list but a collapsed inheritance layer names (`inheritance[].members`, an interned name list in the canonical JSON) is a `layer`: surfaced, but with no type to judge, so it is neither credited as correct nor counted as missing. Each verdict keeps which methods stood behind it (`both`, `checker`, `declaration`), so a count of correct props says how many were corroborated by B and how many rest on A alone.

The sample is every prop a `--list` file names (`[{ corpus, file, component, prop }]`, for example the props two modes differ on) plus a seeded control sample, 100 per corpus by default, stratified so it is not all `string` and `boolean`. A prop falls in the first of these it fits: listed, generic, mapped (`Pick`, `Omit`, `Partial`, …), intersection, function, union of literals, from a dependency, an `@types/react` attribute (at most three per component), from another project file, own. With a tool's output given, the control sample is drawn from the components the tool reports, because the checker takes any exported one-parameter function for a component; pass `allComponents` in the config to draw from all. The candidates are capped at `universeCap` (8,000 by default), taken from the corpus's files in a seeded order, so a corpus larger than that is sampled from part of it; the number collected is printed. A listed prop the checker cannot find (a wrong component name, a file not in the list, a prop it does not see) is judged nowhere, so the report names it (`listed.missing`) beside how many were requested and found, and counts the entries that name a corpus the run does not have (`listed_for_other_corpora`).

```bash
scripts/reference_sample.py --bin target/release/caliper-docgen --corpus antd --corpus shadcn [--per-corpus 300] [--list diff.json]
```

The report (`reference.json`) counts, per corpus, how the reference was reached (`agreed_both`, `checker_only`, `declaration_only`, `contested`, `none`), why B abstained, and the tool's outcomes in total, by stratum and by origin. `review.md` lists the contested props with both types and the declaration's path; `reference.json.rows.json` has every judged prop. Output goes to `--out-dir`, a temporary directory by default.

What it does not establish:

- A and B read the same declaration. They corroborate each other where B can read a type, and say nothing where it cannot (a computed type, which is where a checker-backed mode matters most). Count `checker_only` as props with no independent evidence; a labelled sample is the independent check for those.
- `oracle-compare.ts` reads a widened primitive of another kind as `less precise`, not `wrong` (`number` against `'a' | 'b'`). Changing that would move every historical oracle number, so the reference inherits it.
- The sample is the components Caliper reports, so it says nothing about components Caliper misses.

### Layer-only props

**Which configuration this describes.** The CLI, the NAPI binding and the Vite plugin collapse by default (`collapseInheritedProps: 'dependencies'`). The Storybook addon does not: it defaults to `'none'` (`packages/storybook/src/provider.ts`), which lists every inherited prop with its type, so in a Storybook props table no prop is layer-only. What follows describes what a default CLI, NAPI or agent consumer gets; the addon's configuration is the fully expanded run, reported beside it.

A collapsed inheritance layer (a dependency source declaring more than 30 props, `docs/architecture/resolver-type-resolution.md`, "Collapse") reaches a reader as a list of names, `inheritance[].members`, with no types. `scripts/layer_only_props.py` (`apps/oracle/src/layer-only.ts`) takes every such prop that the component's `props` and `notableInherited` do not list, over all components of a corpus, and sorts it into two classes by a rule that reads only the layer's file, the layer's type name and the prop's name. The first clause that fits decides:

1. **dom file** → ambient. The layer is declared in `@types/react`, or in react-aria's `@react-types/shared/src/dom.d.ts`.
2. **dom type** → ambient. The layer's type is `GlobalDOMEvents`, `DOMAttributes`, `AriaAttributes`, `AllHTMLAttributes`, `SVGAttributes`, `*HTMLAttributes` or `Aria*Attributes`.
3. **style type** → ambient. The layer's type is `*StyleProps`, `SystemProps`, or `Space|Layout|Color|Typography|Border|Flex|Grid|Position|Shadow|Background` followed by `Props`.
4. **global attribute** → ambient. The prop is one every element has (`GLOBAL_ATTRIBUTES` in `layer-only.ts`: `className`, `style`, `id`, `title`, `lang`, `dir`, `hidden`, `tabIndex`, `role`, `slot`, …), or starts `aria-` or `data-`. `value`, `defaultValue`, `type`, `name` and `disabled` are not in it: they are element-specific, and a dependency's component often uses them as its own API.
5. **dependency api** → API. Anything else a collapsed layer names: the modes, state and composition props of a dependency's component.

The 20 API props that matter most are chosen by category first (state, then mode, composition, constraint, data, other, by the prop's name), then by the number of components it is layer-only in. The report gives each its declaring layer, its declared type (read from the layer's `.d.ts`), and why it matters.

```bash
scripts/layer_only_props.py --bin target/release/caliper-docgen --corpus antd --corpus shadcn [--runs 3]
```

Measured at the commit that added this, on antd and shadcn (Mantine, Chakra and Spectrum are not run):

|                                                     |      antd |      shadcn |
| --------------------------------------------------- | --------: | ----------: |
| Layer-only props (component × prop)                 |       669 |       6,774 |
| API / ambient                                       |   664 / 5 | 267 / 6,507 |
| Distinct (layer, prop), API / ambient               |   320 / 3 |     89 / 69 |
| Components with at least one API layer-only prop    | 22 of 253 |  6 of 1,901 |
| Collapsed layers (all above 30)                     |        10 |           3 |
| … of which 31 to 40 props / 41 to 60 / 61 and over  | 5 / 4 / 1 |   1 / 0 / 2 |
| … of which hold API props / all ambient             |    10 / 0 |       2 / 1 |
| API props in a layer of 31 to 40 props              | 240 (36%) |    81 (30%) |
| Dependency sources listed in full (all 30 or fewer) |        46 |         314 |
| … of which ambient by clause 1 to 3                 |         0 |          13 |

shadcn's ambient mass is one layer: react-aria's `GlobalDOMEvents`, 64 handlers on 106 components. Its API props are react-day-picker's `PropsBase` (75 props) and Base UI's `ComboboxRootProps` (38). In antd, nearly everything collapsed is a dependency's API: `@rc-component`'s `IDialogPropTypes`, `BaseSelectProps`, `InputNumberProps`, `MenuProps` and the like, so with the default collapse a CLI, NAPI or agent consumer gets `visible`, `onClose`, `value`, `defaultValue` and `onChange` as names, with no type. The addon's `none` lists them with their types.

What this says about the size rule: it is right where a source happens to be both large and ambient, and wrong where it is large and API. A source of 31 props is collapsed and one of 30 is listed, whatever is in it; 36% and 30% of the API props sit in the 31 to 40 band. It also lists small ambient sources in full (13 in shadcn).

**Proposal: collapse by relevance, not size.** Collapse a dependency source when it is ambient by clause 1 to 3, decided from the source's name and file before anything in it is resolved, whatever its size. Do not collapse a source for being large: list its members, keeping clause 4's global attributes as a name list. This changes the resolver (`collapse_inherited_props`), which this measurement does not.

Who it is for: the default CLI, NAPI and agent consumers, which collapse by size today and so hold the API props above as names. The Storybook addon already lists everything (`none`) and would see no change in its API props; it could adopt the rule to stop listing ambient names, which the "expanding every layer" row prices in output size but which was not measured as a session's payload.

Its measured effect, from the run with `--collapse-inherited-props none` (the canonical JSON's size; the listing counts are exact, the bytes an estimate from the distinct entries plus one name per component):

|  | antd | shadcn |
| --- | --: | --: |
| API props it would list | 603 (313 entries, about 199 KB) | 255 (85 entries, about 107 KB) |
| Ambient props (clauses 1 to 4) from dependency sources it would collapse that are listed today | 37 (about 16 KB) | 2,896 (about 297 KB) |
| Net change to the output | about +183 KB (+4.6% of 3.95 MB) | about −190 KB (−2.2% of 8.63 MB) |
| Expanding every layer instead (the Storybook addon's `none`: 0 collapsed layers, 0 layer-only props) | +189 KB (+4.8%); 19,889 component props listed, not 19,281 | +837 KB (+9.7%); 396,864 listed, not 390,102 |
| Time and peak RSS, default / `none` (fastest of 3, interleaved) | 66 / 74 ms, 116 / 117 MiB | 170 / 177 ms, 295 / 293 MiB |

Time and memory are the size rule's reason (Chakra went from 560 to 398,910 props with its source directory's style system listed), so this does not show the proposal is safe for a corpus with a large dependency API or a large style system: clause 3 keeps a style system collapsed by its type, but a dependency whose own API is hundreds of props would be listed in full. That needs the same run on Chakra, Mantine and Spectrum, which are not installed here.

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
