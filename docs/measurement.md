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

Caliper collapses an inherited dependency source of more than 30 props into a layer (`docs/architecture/resolver-type-resolution.md`, Collapse) and lists only its notable members as props, as Storybook does. A prop in a layer is in the output, named by the layer's `members`, but is not in the flat list, so the table reports both: the plain columns say what the default output lists, "with layers" says what it holds. react-docgen writes no layers, so its two columns are equal.

react-docgen parses each file with its default resolver and importer, as Storybook does. The importer follows relative and package imports to the file that declares a type, so a props interface imported from a sibling file is read. It does no type inference: it lists none of React's inherited attributes, and a generic, mapped or conditional type stays the source text. That is the point of the comparison. A file it throws on is counted (`failed`) and listed (`failedFiles`), not dropped silently.

The run uploads `weekly.json` and a table to the job summary, which opens with a short "how to read these numbers" paragraph and the headline (below). It compares with the last successful run's `weekly.json` and opens, or comments on, one issue labelled `weekly-measurement` only for what is a problem on its own (see "The headline"): a wrong type above the gate or higher than last week, a silent-wrong answer to a reader's question, or a whole run over a budget. Drift in time, memory, recall against RDT and the agent correct rate is listed in the summary under "Changes since last week" and opens nothing: a shared CI machine moves by 10% or more on its own, and recall against RDT is how much of RDT's output we also report, not whether our types are right. There is no committed baseline: the previous run is the baseline for what rose.

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

- **Agreement of the whole universe** (`agree_universe`): identical or spelled, over every prop in the universe. A prop the tool omits counts against it. This is the comparison between tools; Caliper's own headline is below.
- **Agreement of listed props** (`agree_listed`) and the share the checker contradicts (`wrong_listed`): over the props the tool lists, which is what Caliper's own `agree` and `wrong` have always been.
- **Omitted props**, by where the checker declares them (`missing_by_stratum`): the component's own file, another file of the corpus, or a dependency. A prop Caliper lists only as a member of a collapsed inheritance layer is counted apart (`collapsed`), because that is a choice and not an omission.
- **Why a prop is omitted** (`missing_by_reason`): the tool reports no such component, it reports the component but not the prop, or it threw on the file.
- **react-docgen's thrown files** (`failed_files`): files its default resolver throws on, 17% to 21% of the files on antd and shadcn. Their props are omitted, so they cost react-docgen agreement; they have their own column so the cost is visible.

`oracle.shared_props`, `agree` and `wrong` keep their meaning, so the weekly history does not move. A local run on antd and shadcn at the commit that added this reads, for the whole universe: Caliper 39.7% and 40.1% (its `collapsed` layers and any component it does not report count against it), RDT 44.8% and 84.8%, react-docgen 24.6% and 1.9%. This comparison between tools is information, never a gate, and the rules are validated on antd and shadcn only: Mantine, Chakra and Spectrum need a weekly run.

### The headline

What matters is whether the types Storybook and agents are shown are right, not whether Caliper matches RDT. The headline (`scripts/headline.py`, read from `oracle.tools.caliper_reported`, which `apps/oracle/src/tool-score.ts` `scoreReported` computes) takes every own prop the checker finds on a component Caliper reports, so a function the checker takes for a component and Caliper rightly ignores does not count against it, and sorts each prop into one kind:

- **Correct**: the same type as the checker, or the same type spelled differently.
- **Less precise**: vaguer than the checker's (a name left unexpanded, `string` for a union of literals), not contradicting it.
- **Wrong**: describes something else. This is the gate.
- **No checker type**: the checker has none either (an unresolved import, `any`).
- **Missing**: Caliper reports the component but not the prop.
- **Named only**: Caliper lists the prop by name as a member of a collapsed inheritance layer, so it is surfaced but a reader sees no type for it. It is counted apart, as neither correct nor wrong, because correct values reached through a layer are a win and a bare name is not.

The record keeps the counts per corpus as `headline`, next to the unchanged `oracle` and `recall` fields. The shares in the summary are of every prop of the components Caliper reports. The gate uses the wrong share of the props whose type can be judged (correct, less precise and wrong), so a corpus with many layer-only props does not dilute it.

The issue opens when, on any corpus:

- more than 0.1% of the judged props are wrong, or the wrong share is higher than last week's;
- a question a reader is asked is answered silent-wrong (`scripts/agent_eval.py`);
- a whole run is over a budget: 2 s or 600 MiB with dependencies loaded, the default (`BUDGETS` in `scripts/headline.py`). The budgets are user-facing limits, not week-over-week ones; Chakra, the largest corpus, runs in about 1 s and 446 MiB.

Recall against RDT stays information. A local run on antd and shadcn at the commit that added this reads: antd 83.7% correct, 6.0% less precise, 0.00% wrong, 10.2% named only (3,426 props); shadcn 54.4% correct, 5.1% less precise, 0.06% wrong (0.096% of the judged props, just under the gate), 0.4% missing, 39.6% named only (13,918 props). Mantine, Chakra and Spectrum need a weekly run.

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

## `just bench`

```bash
just bench                 # this checkout, uncommitted work included, against origin/main
just bench my-branch       # a branch
```

`scripts/bench.py` builds both in release mode, then times `extract --json` over Mantine and Chakra in the Storybook addon's configuration (dependencies loaded, cache on after the warm-up run, inherited props not collapsed) over corpora with their dependencies installed with the two builds interleaved and alternating which goes first. It prints the median wall time and peak RSS of each and their ratio. It stops with the setup command to run when a corpus is not installed, because a sparse fetch has no dependencies to load: run `scripts/corpora.py setup --corpus mantine --corpus chakra` first.

The 1.10× and 1.20× thresholds come from a calibration made when the bench ran with `--no-cache --no-load-dependencies` (with nothing changed, the ratio stayed inside 1.04 for time, 95th percentile of 100 trials per corpus on CI and a laptop, and inside 1.12 for memory on CI runners). The bench now runs the addon's configuration (dependencies loaded, warm cache) and that calibration has not been repeated for it, so the noise band may differ. More than 1.10× in time or 1.20× in memory is a starting point for looking. Nothing enforces that. Run it before a resolver change goes up for review, and say in the pull request when a cost is intended.

## Checker modes

`scripts/checker_modes.py` measures the experimental TypeScript 7.1 checker modes against the structural default on the corpora `scripts/corpora.py` sets up. It is run by hand for now and is not part of the weekly record.

```bash
cargo build --release -p caliper-docgen && pnpm --filter caliper-docgen-napi build:napi
python3 scripts/checker_modes.py --bin target/release/caliper-docgen --runs 3 --count 50 --fallbacks \
  --out report.json --diff-out diff.json --pairs-out pairs.json
```

What it runs, per corpus: the structural default (`extract --format rdt`), the same with `--experimental-semantic-types` (the CLI's whole-project checker pass), and one lazy session driven the way the Storybook addon drives it (`scripts/checker_modes_session.mjs`: `createSession`, `initializeSessionIndex`, then `sessionComponentAnswers` per component) with the checker off and with `semanticTypes: 'on-demand'` at a 150 ms and a 600 ms wait. `--fallbacks` also runs the CLI and a session against a missing `tsc` and one that exits at once.

The settings are the Storybook addon's (`createDocgenProvider` in `packages/storybook/src/provider.ts`): dependency types loaded, `collapseInheritedProps: 'none'`, a lazy session, and the NAPI defaults for the rest (`htmlAttributes: 'full'`, the on-disk cache on). The CLI runs pass the same props settings (`--load-dependencies --collapse-inherited-props none --html-attributes full`) so that the structural and the checker run list the same props. A test reads the addon's source and fails if those defaults change. What the sessions leave out is the per-story look at the files (`ensureFresh`, a few milliseconds on a large project) and `warmUp`/`fillAfter`, both off in the addon by default.

What it reports:

- Wall time and peak RSS, median with min and max over `--runs` runs, and the machine's load average before and after. Peak RSS is the largest single process (`corpora.run_once`), so a CLI checker run reports the larger of Caliper and `tsc`. A session reports whole-process figures only with the checker off: an on-demand run sleeps before asking the same components a second time, so the time and memory of that script mean nothing next to the checker-off run, and its own step timings are reported instead. The checker's size comes from polling `ps` from the parent while the session runs, never from inside the session, because a measurement between two requests would give a slow checker idle time that the timings do not count. The checker is the direct child of that run's session driver that is a `tsc` in `--api` mode. It is found by what it is, not by a string in a command line: the driver's and the measuring helper's own command lines carry the `tsc` path in their JSON options, and either can use more memory than the checker. The path is not compared either, because the configured `bin/tsc` is a launcher and what runs is the platform binary it resolves to (`@typescript/typescript-<platform>/lib/tsc --api --async --cwd <dir>`). When no such process is seen (a missing `tsc`, or `ps` gave no listing) the size is `None` and prints as n/a, never 0.
- First load for a session, one request at a time in a seeded order that is the same in every mode: start (create plus index), the first component, and the first 10 and 50 components asked. A figure for more components than were asked is `None`, not the last one reached. The first of the `--runs` runs starts with an empty cache directory (cold), the others with the cache it left (warm), and both are reported.
- For the on-demand mode: the share of first requests the checker answered within the wait, how many timed out, how many kept the structural result for the checker's own reasons (not exported, a class component, generic props), and the share answered once the late answers had arrived.
- The props that differ from the structural result: type, required, default and presence. `@types/react` props are counted apart (the whole-project checker pass leaves them out), and a type that differs only in spelling (spacing, `| undefined`, union order, `Array<T>`) is counted as cosmetic. A rest or optional parameter marker is a difference. Each remaining difference gets a coarse category: `only_checker`, `only_structural`, `parameter_marker_lost`, `checker_longer` or `checker_shorter` (the checker's text is more than 20 characters and more than 1.5 times longer or shorter, a length test that says nothing about why, so a longer text may be an alias written out, an object type or a function type), and `other_type`. `--diff-out` has one row per differing prop (without `@types/react` and cosmetic rows, unless `--diff-all`), `--pairs-out` one entry per distinct pair of structural and checker facts with a count, a category and an example, most common first.
- The checker oracle's verdict on each mode, for information only: the shares of shared props it calls identical or spelled differently (agree), less precise, and wrong. The oracle is the same checker, so it agrees with a checker mode almost by construction, and it credits the structural result for keeping a type's alias name while it scores the checker's expanded union as less precise (`docs/type-checker-integration.md`). Do not read that agree figure as accuracy; judging the checker modes needs a reference that is not the checker. The oracle step runs the CLI as the weekly record does, with its own flags and the default collapse.

### Results, 2026-11-02 (second run, under the addon's settings)

Measured by hand on `main` at `c267c777`, on a laptop whose load average ran from 20 to 170 while other jobs were building and measuring, on local checkouts of the five repos that are not at the commits `scripts/corpora.py` pins. The absolute numbers are 1.5 to 3 times worse than an earlier run of this script on a quieter machine, so compare ratios within this run and nothing across runs, and do not compare with the weekly record or with `docs/performance.md`. The Spectrum oracle figure (11.79% wrong, against 1.09% in the weekly record) is from a different checkout and is unverified. The time and memory figures use `--format rdt`, which does not intern props, so they are larger than the weekly record's `--json` figures. Re-run on the pinned corpora and an idle machine before deciding anything.

| Corpus | Structural | Whole-project checker | First 50, checker off (warm) | First 50, on-demand 150 ms (warm) | Checker answered in time (150 ms) |
| --- | --- | --- | --- | --- | --- |
| mantine | 417 ms | 2.0 s (4.8 times) | 275 ms | 3.9 s | 96% (92% cold) |
| chakra | 3.8 s | 25.6 s (6.7 times) | 684 ms | 8.2 s | 0% |
| shadcn | 2.6 s | 4.4 s (1.7 times) | 217 ms | 3.6 s | 92% |
| antd | 154 ms | 1.0 s (6.7 times) | 167 ms | 2.5 s | 77% |
| spectrum | 281 ms | 1.9 s (6.8 times) | 46 ms | 2.4 s | 96% |

- **First load, checker off:** session start 47 to 149 ms, the first component 0 to 103 ms, 10 components 24 to 216 ms and 50 components 46 to 684 ms (Chakra is the high end), the whole process peaking at 198 to 446 MiB with Node and the binding. A cold and a warm cache show no consistent difference under this load.
- **On-demand checker:** the first answer takes 47 to 117 ms (256 to 273 ms on Chakra, where it timed out). 50 sequential requests cost 2.4 to 4.0 s on four corpora and 8.2 s on Chakra: about 50 to 80 ms per component (160 on Chakra), against 46 to 275 ms for all 50 without the checker. A request the checker has not answered waits the whole 150 ms before it takes the structural answer. Chakra's checker answered none of the first requests, at 150 ms or at 600 ms (48 timeouts in each), and 15 to 21% of them when asked again at 150 ms and 67 to 83% at 600 ms. The checker's own peak was 227 to 345 MiB (300 to 306 MiB on Chakra), measured in a third run after the sampler was fixed to find the checker process itself; an earlier version of this script reported the Node driver's size as the checker's, so a "177 to 340 MiB" (Chakra "566 to 731 MiB") figure that appeared before is void. That third run, at a lower load, took 1.4 to 3.2 s for the first 50 on-demand requests at 150 ms on four corpora (Chakra 7.8 s) and again answered none of Chakra's first requests, so the 2.4 to 4.0 s above is partly load.
- **The fallbacks work:** a missing `tsc` and a `tsc` that exits at once each give exit code 0, output byte-identical to the structural result, and one `TypeScript checker backend unavailable` warning, in the CLI and in a session. A hanging or slow `tsc` is covered by the tests in `crates/semantic`.
- **What differs, whole-project checker, props that are not `@types/react` and not spelling only, all five corpora:** 499,241 have a much longer checker text (Chakra's `ConditionalValue<SpacingToken | ...>` and Mantine's `StyleProp<MantineSpacing>` read as their members, but the category is a length test and the script does not parse why), 6,069 other type text, 2,968 props only the checker lists, 2,467 a much shorter checker text, 1,273 props only structural lists, and 71 a rest or optional parameter marker the checker's text does not carry (`(...args: any[]) => void` becomes `(args: any[]) => void`, `(percent?: number) => ReactNode` becomes `(percent: undefined | number) => ReactNode`). Required and default never differ.
- **Props only the checker lists** are Spectrum and antd components that lack DOM event handlers (`onAuxClick`, `onGotPointerCapture`, `onClick`) in the structural output, and Mantine's `__size`, `__vars` and `__clearable`, which the checker names `___size`, `___vars` and `___clearable`: TypeScript's internal escaping adds an underscore and the checker mode does not take it off. With the addon's settings shadcn has only 63 props the checker alone lists; an earlier run with the CLI's default collapse showed 6,303 there, because the DOM props sat in a collapsed layer on the structural side.
- **The oracle**, as information: the share it calls less precise is 0.0 to 9.1% for the structural result and 17 to 84% for the whole-project checker (mantine 77.8%, chakra 83.6%, shadcn 49.2%, antd 17.1%, spectrum 69.5%), because it scores a type the checker writes out as less precise than one kept by name. Wrong is 0.26% or lower for the checker, and 0.0 to 0.10% for the structural result (11.79% on the unverified Spectrum checkout).

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
