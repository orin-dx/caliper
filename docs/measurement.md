# Measurement

Two things, both small: a weekly record of how Caliper compares with react-docgen-typescript (RDT) and react-docgen on five public repos, and `just bench` for a change that touches the resolver. Nothing here blocks a pull request.

## Weekly record

`.github/workflows/weekly.yml` runs on Mondays and by hand. `scripts/corpora.py` fetches each repo at a pinned commit and installs its dependencies with its own package manager and lockfile (lifecycle scripts off, caches under `.corpora/`, gitignored), then removes tests, stories, demos and the `dist/` and `build/` directories from the source directory so every tool reads the same files. `scripts/weekly.py` runs the tools.

Declaration files are kept. Until 2026-11 the fetch also deleted every `.d.ts`, which removed a corpus's own type packages: React Spectrum declares `@react-types/shared` in `packages/@react-types/shared/src/*.d.ts` and its components import from it. With those files gone, 20 props that reach a component through `Pick` of a name declared there were lost; on unmodified main with the files restored, Caliper reports 20 of 20. Generated declarations sit in `dist/` and `build/`, which are still removed. A fetched corpus records a prune version in its marker (`PRUNE_VERSION` in `scripts/corpora.py`), so one pruned under the older rule is fetched again on the next `setup`; `--sparse` never pruned and already kept declaration files. The first weekly run after this change moves Caliper's and RDT's numbers on every corpus whose source tree holds `.d.ts` files (React Spectrum has 67 under `packages/`).

| Corpus | Repository | Commit | Source directory | Install |
| --- | --- | --- | --- | --- |
| mantine | `mantinedev/mantine` | `e47e45af06427b3b2497100c2b849935c23af30d` | `packages/@mantine/core/src` | yarn, immutable |
| chakra | `chakra-ui/chakra-ui` | `fa9696f328ea5a0df8685c6d8460edbd779b63b9` | `packages/react/src` | pnpm, frozen |
| shadcn | `shadcn-ui/ui` | `ba1503c64472b82256d9de95e2e62c5901a38e8b` | `apps/v4` | pnpm, frozen |
| antd | `ant-design/ant-design` | `ba7ccbaa7b4b58ac1ff8e6fe2e78dfa10bfb5538` | `components` | npm, no lockfile (the repo commits none) |
| spectrum | `adobe/react-spectrum` | `797762ed3526ea12f089e321bd3891066affad70` | `packages` | yarn, immutable |

It records, per corpus and tool, the wall time and peak RSS of one whole run (process start and Node start included), and the size of Caliper's canonical JSON. Caliper runs with `--load-dependencies` and `--no-checker`; its time is the median of five runs. Since ADR 0026's S8 the checker answers by default, so the record (time, memory, recall and the oracle's verdicts) is the structural resolver's, the fallback, and its budgets (2 s, 600 MiB) are that resolver's: the checker's whole-project run takes seconds on a large program (`docs/gate-measurements.md`, "The checker is the default"). Recording the checker weekly needs budgets of its own first. RDT and react-docgen run once each.

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

The 14 antd items still unpaired in the weekly run of 2026-11-06 (16 when this was first written) are not pairing cases: the date picker and calendar families (built by factory functions), `Parent.Member` entries Caliper reports under the parent (`Form.List`, `Mentions.Option`, `TreeSelect.TreeNode`), and helpers RDT lists as components (`confirm`, `dropIndicator`). shadcn has no re-export of this kind in the pinned commit, so nothing pairs there; its unpaired items are `Icons.logo`-style object members, aliases such as `const ComboboxRoot = Primitive.Root` and factory results, which Caliper does not detect.

Recall asks how much of RDT's reference Caliper also reports: the share of RDT's reference components and props that Caliper also reports. It is not a measure of correctness, and a component or prop RDT lists that Caliper does not is not necessarily right. TypeScript's checker is the arbiter of a prop's type ("The headline" below), and RDT is judged by it too. On the props both tools list, in the weekly run of 2026-11-06 (`docs/rdt-differences.md`), the checker contradicts Caliper on 0, 0.0002%, 0.058%, 0 and 1.09% of them (Mantine, Chakra, shadcn/ui, Ant Design, React Spectrum) and RDT on 0.04%, 0.006%, 0.59%, 1.05% and 5.02%. The Caliper figures are the comparator's `wrong`, which the headline section says both over- and under-counts. Where Caliper reports less than RDT, it often declines to guess (class 9, opaque with a marker). Where RDT reports more, some of it is not a component (class 16), and RDT's types are truncated on some props (class 8).

Recall is measured against that reference, on the components both tools report (same file, same name, or paired):

| Measure              | Meaning                                                                                   |
| -------------------- | ----------------------------------------------------------------------------------------- |
| Components           | The reference's components that the tool also reports                                     |
| Own props            | Of those components, RDT's props not declared by `@types/react` that the tool lists       |
| `@types/react` props | RDT's props that `@types/react` declares (`onClick`, `aria-*`, `ref`) that the tool lists |
| ... with layers      | The same two, also counting a prop held in a collapsed inheritance layer                  |

By default (`collapseInheritedProps: dependencies`) Caliper collapses a large ambient inherited dependency source (DOM and ARIA attributes, style systems; more than 30 props) into a layer (ADR 0012; `docs/architecture/resolver-type-resolution.md`, Collapse) and lists only its notable members as props. A dependency's own API is listed in full (a source past 200 props that is not ambient by name still collapses). The Storybook addon runs with `collapseInheritedProps: none`, which lists every inherited prop; the layers below are what the CLI, the NAPI binding and agents reading the output see. A prop in a layer is in the output, named by the layer's `members`, but is not in the flat list, so the table reports both: the plain columns say what the default output lists, "with layers" says what it holds. react-docgen writes no layers, so its two columns are equal.

react-docgen parses each file with its default resolver and importer, as Storybook does. The importer follows relative and package imports to the file that declares a type, so a props interface imported from a sibling file is read. It does no type inference: it lists none of React's inherited attributes, and a generic, mapped or conditional type stays the source text. That is the point of the comparison. A file it throws on is counted (`failed`) and listed (`failedFiles`), not dropped silently.

The run uploads `weekly.json` and a table to the job summary, which opens with a short "how to read these numbers" paragraph and the headline (below). It compares with the last successful run's `weekly.json` and opens, or comments on, one issue labelled `weekly-measurement` only for what is a problem on its own (see "The headline"): a wrong type above the gate or higher than last week, a silent-wrong answer to a reader's question, or a whole run over a budget. Drift in time, memory, recall against RDT and the agent correct rate is listed in the summary under "Changes since last week" and opens nothing: a shared CI machine moves by 10% or more on its own, and recall against RDT is how much of RDT's output we also report, not whether our types are right. There is no committed baseline: the previous run is the baseline for what rose.

### TypeScript's checker as the oracle

RDT has mistakes of its own. With `--oracle` the run also asks TypeScript 7.1's checker (`apps/oracle/src/oracle.ts`, `typescript-beta/sync`, the `7.1.0-beta` package) for each prop's type and compares it structurally with Caliper's canonical `--json`, extracted with `--html-attributes full --include-unexported --load-dependencies --collapse-inherited-props none`. The weekly table shows the share of props that agree (identical, or the same type spelled by name) and the share the checker contradicts, for Caliper, RDT and react-docgen, and the headline below. `agree` is the loose number; `wrong` is the one that matters and the one the weekly issue gates on, but the comparator has artifacts that make it both over- and under-report. `docs/rdt-differences.md` lists every class of disagreement with an example, how it is counted and whether the comparator scored it right. The checker is a beta, so a disagreement is a reason to look, not a verdict on its own. The weekly extracts with `--no-checker`, so this judges the structural resolver, the fallback since the checker became the default (`docs/type-checker-integration.md`).

| Verdict | Meaning |
| --- | --- |
| Identical | Both sides describe the same type |
| Spelled | The same type, Caliper by name where the checker expands it |
| Less precise | Caliper's type is looser or left unevaluated |
| Wrong | Caliper's type describes something else (an omitted key that came back, an object with no fields for a callable) |
| Checker any/error | The checker has no type |
| Not judged | The oracle did not evaluate the checker's type (cut at the depth limit, or a kind `oracle-checker.ts` does not read), so nothing was compared |

A not judged prop is neither agreement nor a difference. Before `oracle.universe_version` 4 the comparison scored it identical, so a type the checker side never read counted as right. A part of a type that is not judged (an array of an unevaluated element) makes the whole not judged unless another part already differs, in which case the difference wins. The verdict names, their order and which of them agree are one definition, `apps/oracle/src/vocabulary.ts`, written to `apps/oracle/src/vocabulary.json` (`pnpm --filter caliper-docgen-oracle vocabulary`) for `scripts/weekly.py` and `scripts/headline.py` to read; `apps/oracle/tests/vocabulary.test.ts` fails while the two differ. Caliper's own side (`apps/oracle/src/caliper-norm.ts`) is typed against the published `PropType` (`packages/napi/index.d.ts`) with a case for every kind, so a kind added there fails the type check, and a kind a build does not know throws instead of comparing as opaque.

#### All three tools against the checker

RDT and react-docgen are held to the checker too, by the same rules, so the comparison does not take either as the truth. `oracle.ts` also takes the dumps `scripts/rdt.mjs` and `scripts/rdg.mjs` write (`--rdt`, `--rdg`). Both tools state a type as source text, which `apps/oracle/src/tool-norm.ts` reads with TypeScript's parser into the shape the comparison uses; a type a tool printed in part (`Foo<...>`, `{string}`) or did not evaluate is unreadable, which scores as less precise and never as wrong. The scoring is `apps/oracle/src/tool-score.ts`.

The **universe** is every own prop the checker finds on an exported function whose only parameter, or `props` parameter, is the props type, and which can be a component. React's inherited DOM attributes, `ref` and `key` are not in it. The checker has no notion of a component, so a function is left out when its shape says it is none, by the rules that clean the reference for recall (the table above): more than half of its props are declared in TypeScript's `lib.*.d.ts` (the parameter is an `Element` or a `String`), its name is a hook's, its only props are a context Provider's `value` and `children`, its name starts lowercase and is no tag's (not the file's or directory's name, no `@types/react` prop), or every prop is a `Promise`. The rules are one definition: `apps/oracle/src/component-shape.json` holds the patterns, `apps/oracle/src/component-shape.ts` applies them to the checker's functions, `scripts/weekly.py` `exclusion` to RDT's entries, and both run the cases of `apps/oracle/tests/fixtures/component-shape-cases.json`, so they cannot drift apart. The paths those rules read, and the oracle's own (where the checker declares a prop, which declarations are `@types/react`'s), are classed by one file, `apps/oracle/src/path-classes.json` (a dependency is anything under `node_modules`, `@types/react` and TypeScript's `lib.*.d.ts` are told apart, `\` is read as `/`): `apps/oracle/src/path-class.ts` and `scripts/oracle_data.py` read it, and `apps/oracle/tests/path-class.test.ts` and `scripts/test_path_classes.py` run the same cases of `apps/oracle/tests/fixtures/path-class-cases.json`. `scripts/rdt.mjs` reads it too, to leave `@types/react`'s props out of its dump. A function Caliper itself reports as a component stays in the universe whatever its shape. A default export is named from its declaration (`export default function Foo` is `Foo`; one with no name of its own takes its file's, PascalCased, as Caliper names it), which is how it meets Caliper's and RDT's entries. A component is its declaration, not an export name: `export default function Foo` followed by `export { Foo }` or `export { Foo as Bar }` is one component, counted once under the declaration's own name, and a re-export (`export { Foo } from './Foo'`, `export *`) is counted only in the file that declares it, never in the importing file; two components of one name in different files are two (`declaredExports` in `apps/oracle/src/checker-components.ts`, which both the oracle and the reference universe read). The universe still holds some functions that are not components, because the rules read shape and not intent, so read the shares as a comparison between tools, not as a share of real components. For each tool the record has, in `oracle.tools`:

- **Agreement of the whole universe** (`agree_universe`): identical or spelled, over every prop in the universe. A prop the tool omits counts against it. This is the comparison between tools; Caliper's own headline is below.
- **Agreement of listed props** (`agree_listed`) and the share the checker contradicts (`wrong_listed`): over the props the tool lists, which is what Caliper's own `agree` and `wrong` have always been.
- **Omitted props**, by where the checker declares them (`missing_by_stratum`): the component's own file, another file of the corpus, or a dependency. A prop Caliper lists only as a member of a collapsed inheritance layer is counted apart (`collapsed`), because that is a choice and not an omission.
- **Why a prop is omitted** (`missing_by_reason`): the tool reports no such component, it reports the component but not the prop, or it threw on the file.
- **react-docgen's thrown files** (`failed_files`): files its default resolver throws on, 17% to 21% of the files on antd and shadcn. Their props are omitted, so they cost react-docgen agreement; they have their own column so the cost is visible.

**What changed on 2026-11-10 (`oracle.universe_version` 2).** Until then the universe counted every such function, and a default export was keyed `default`. On the 2026-11-09 weekly, whole-universe agreement on react-spectrum read Caliper 30.7% against RDT 42.5% of 5,582 props, which read as Caliper being less complete. It was two artifacts, both measured on the saved dump of that run (`apps/oracle/tests/spectrum-universe.test.ts`):

- RDT lists the props of functions that are no component, Caliper rightly does not: 874 props of 65 functions (42 lowercase helpers and 23 hooks), of which RDT agrees on 684 and Caliper lists none. 741 of them are declared in dependencies: 725 members of a lib type (`Element`, `String`) that the parameter takes, the rest a `@parcel/rsc` type. Without these 874, 4,708 props, Caliper agrees on 36.4% and RDT on 35.9%. The gap was RDT listing them.
- The oracle keyed 535 components `default` (529 that Caliper reports under their own names, 6 it does not report), so neither tool's entry met theirs and both scored zero on their 2,613 props. Without those as well, 2,095 props, Caliper agrees on 81.8% and RDT on 80.6%. That figure leaves the `default` props out; what they read once named comes with the next weekly run, which was not repeated for this change.

With the shape rules themselves over the same dump (`default` kept, as it is named now), 4,718 props: Caliper 36.3%, RDT 36.0%. The rules take out 730 props of lib-typed parameters, 58 of hooks and 76 of lowercase helpers, and keep the 10 props of lowercase functions that carry their file's name. The rest of the gap to 81.8% is the default exports, which are in the universe now.

`oracle.shared_props`, `agree` and `wrong` keep their meaning, over the props both tools list. Naming default exports lets Caliper's `Foo` meet the checker's `Foo`, so a corpus with default-exported components, spectrum above all, has more shared props than before (2,023 in the record of 2026-11-06); no other change reaches them. A record without `universe_version` is version 1: its `tools` count every function, with a default export named `default`. A record of version 2 keeps the scores over every function, default exports named, in `oracle.tools_all_functions`, and counts what was left out in `oracle.universe_excluded` (functions and props, by rule). The summary prints both, and the weekly does not read a rise in wrong types, or any other number of the oracle, against a record of another version.

A local run on antd and shadcn at the commit that added the whole-universe comparison, over every function (version 1), with the default collapse (the weekly record collapses nothing, and reads 44.7% and 90.4% for Caliper), reads, for the whole universe: Caliper 39.7% and 40.1% (its `collapsed` layers and any component it does not report count against it), RDT 44.8% and 84.8%, react-docgen 24.6% and 1.9%. This comparison between tools is information, never a gate, and the rules were validated on antd and shadcn; the weekly run produces all five corpora.

### The headline

What matters is whether the types Storybook and agents are shown are right, not whether Caliper matches RDT. The headline (`scripts/headline.py`, read from `oracle.tools.caliper_reported`, which `apps/oracle/src/tool-score.ts` `scoreReported` computes) takes every own prop the checker finds on a component Caliper reports, so a function the checker takes for a component and Caliper rightly ignores does not count against it, and sorts each prop into one kind:

- **Correct**: the same type as the checker, or the same type spelled differently.
- **Less precise**: vaguer than the checker's (a name left unexpanded, `string` for a union of literals), not contradicting it.
- **Wrong**: describes something else. This is the gate.
- **No checker type**: the checker has none either (an unresolved import, `any`).
- **Not judged**: the oracle did not evaluate the checker's type, so nothing was compared. It is in none of correct, less precise or wrong, and is outside the judged props the gate divides by.
- **Missing**: Caliper reports the component but not the prop.
- **Named only**: Caliper lists the prop by name as a member of a collapsed inheritance layer, so it is surfaced but a reader sees no type for it. It is counted apart, as neither correct nor wrong, because correct values reached through a layer are a win and a bare name is not.

The record keeps the counts per corpus as `headline`, next to the unchanged `oracle` and `recall` fields. The shares in the summary are of every prop of the components Caliper reports. The gate uses the wrong share of the props whose type can be judged (correct, less precise and wrong), so a corpus with many layer-only props does not dilute it.

The issue opens when, on any corpus:

- more than 0.1% of the judged props are wrong, or the wrong share is higher than last week's (the count is not trustworthy in either direction yet: the comparator reads an index signature as a property and so reports wrong props that are right, and it scores a dropped `Partial` or `Omit` as less precise or identical, so wrong answers never reach it; `docs/rdt-differences.md`);
- a question a reader is asked is answered silent-wrong (`scripts/agent_eval.py`);
- a whole run is over a budget: 2 s or 600 MiB with dependencies loaded, the default (`BUDGETS` in `scripts/headline.py`). The budgets are user-facing limits, not week-over-week ones; Chakra, the largest corpus, ran in 0.24 s and 378 MiB in the weekly run of 2026-11-06. Only Caliper's own figure for that corpus is read: RDT's and react-docgen's time and memory are references, shown in the table and never in the issue. A peak that is the same to the MiB for Caliper, RDT and react-docgen on one corpus is not read at all (`own_peak` in `scripts/headline.py`): three tools never peak together, so it is one process-wide maximum, which is what the weekly record of 2026-11-02 held (1901 MiB for shadcn, antd and spectrum, every tool), before the helper read each run's own peak. Its first issue (#279) reported that figure and, as its comment, time rises of 11 to 33% on a median of five runs between two records, which the 10% rule of that week read as getting worse; the rule now opens nothing for them (`changes` lists them in the summary).

Recall against RDT stays information. A local run on antd and shadcn at the commit that added this, with the default collapse, reads: antd 83.7% correct, 6.0% less precise, 0.00% wrong, 10.2% named only (3,426 props); shadcn 54.4% correct, 5.1% less precise, 0.06% wrong (0.096% of the judged props, just under the gate), 0.4% missing, 39.6% named only (13,918 props). The weekly record collapses nothing (`--collapse-inherited-props none`), so named only is 0 there; its run of 2026-11-06 reads antd 94.0% correct, 6.0% less precise, 0 wrong, and shadcn 93.8% correct, 5.3% less precise, 8 wrong (0.058% of the shared props), 0.4% missing (`headline` in `weekly.json`, `main` at `c5fbb33e`). Mantine, Chakra and Spectrum read in that run: 90.9%, 100.0% and 83.7% agree, with 0, 1 and 22 wrong. Of Spectrum's 22, the comparator's index-signature artifact accounts for all (`docs/rdt-differences.md`), so that gate firing is false until the comparator is corrected.

**What changed with `oracle.universe_version` 4: an unjudged comparison is scored apart from agreement.** The comparison returned `identical` whenever the checker side was opaque (a type the normalizer cut at the depth limit or has no reader for), so those props counted as agreement. They are now `not judged` (`oracle.verdicts['not judged']`, `tools.*.verdicts['not judged']`, a column of the headline table, and `headline.not_judged`). `shared_props`, `agree` and `wrong` keep their formulas, over the props both tools list: a not judged prop is in `shared_props` and in neither `agree` nor `wrong`. The numbers on the next weekly run therefore change: `agree` falls, the correct share of the headline falls, and the wrong share of the judged props can rise slightly because the judged props are fewer, by as many props as were opaque on the checker side, which has not been measured (this change was not run against the corpora). The weekly does not compare version 4 with a record of version 3, so that first move opens no issue. The construct suite shows the size of it on 4 of its fixtures: through the checker path `generics/19-recursive-json`, `react/16-ref`, `react/27-sx-object` and `react/30-dispatch-setstate` read `identical` against goldens cut at the depth limit, and read `not judged` now (`apps/oracle/construct-suite/checker-expectations.json`). Two smaller moves ride with it: the reference of the next section no longer takes an unevaluated checker type as a reference (the declaration stands alone, or the prop has no reference), and the oracle's three copies of "is this path `@types/react`" and "is this path a dependency" are one rule (`apps/oracle/src/path-classes.json`), which reads an `@types/react` directory outside `node_modules` as the project's own and a Windows path as its POSIX spelling.

**How the oracle pairs a component with Caliper's (`oracle.universe_version` 3, ADR 0027).** A checker component is found once per declaration (a default export and `export { Foo }` of one function are one), and paired with the Caliper component whose canonical JSON `exports` lists that file and one of the names the module exports the declaration under. Until version 3 the pairing normalised display names (path, `.` prefix and every non-alphanumeric character stripped), so `_Card` and `Card` in one file were one name and neither paired, and `Menu.Item` could pair with an `Item` export. RDT and react-docgen name a component only by name, so they are still matched on the name they list it by, as the checker's export is named (`listedName` in `apps/oracle/src/checker-components.ts`), without the normalisation. The weekly numbers move on the first run at version 3 (shared props, agreement and RDT's and react-docgen's scores, mostly where one file has several components or a renamed export); the weekly does not compare them with a record of another universe version.

### Layer-only props

**Which configuration this describes.** The CLI, the NAPI binding and the Vite plugin collapse by default (`collapseInheritedProps: 'dependencies'`). The Storybook addon does not: it defaults to `'none'` (`packages/storybook/src/provider.ts`), which lists every inherited prop with its type, so in a Storybook props table no prop is layer-only. What follows describes what a default CLI, NAPI or agent consumer gets; the addon's configuration is the fully expanded run, reported beside it.

A collapsed inheritance layer (a dependency source declaring more than 30 props, `docs/architecture/resolver-type-resolution.md`, "Collapse") reaches a reader as a list of names, `inheritance[].members`, with no types. A classifier measured every such prop that the component's `props` and `notableInherited` do not list, over all components of a corpus, and sorts it into two classes by a rule that reads only the layer's file, the layer's type name and the prop's name. The first clause that fits decides:

1. **dom file** → ambient. The layer is declared in `@types/react`, or in react-aria's `@react-types/shared/src/dom.d.ts`.
2. **dom type** → ambient. The layer's type is `GlobalDOMEvents`, `DOMAttributes`, `AriaAttributes`, `AllHTMLAttributes`, `SVGAttributes`, `*HTMLAttributes` or `Aria*Attributes`.
3. **style type** → ambient. The layer's type is `*StyleProps`, `SystemProps`, or `Space|Layout|Color|Typography|Border|Flex|Grid|Position|Shadow|Background` followed by `Props`.
4. **global attribute** → ambient. The prop is one every element has (`GLOBAL_ATTRIBUTES` in `crates/core/src/resolver/ambient.rs`: `className`, `style`, `id`, `title`, `lang`, `dir`, `hidden`, `tabIndex`, `role`, `slot`, …), or starts `aria-` or `data-`. `value`, `defaultValue`, `type`, `name` and `disabled` are not in it: they are element-specific, and a dependency's component often uses them as its own API.
5. **dependency api** → API. Anything else a collapsed layer names: the modes, state and composition props of a dependency's component.

The 20 API props that matter most were chosen by category first (state, then mode, composition, constraint, data, other, by the prop's name), then by the number of components it is layer-only in.

Measured once, by hand, on antd and shadcn (Mantine, Chakra and Spectrum were not run); the classifier is not kept, and its rule is `crates/core/src/resolver/ambient.rs`. These figures predate the last change to the count: an `Omit` now drops the omitted props from a collapsed layer's `members` (`resolver/alias.rs`), and `expandedDependencySources` skips collapsed sources that list a curated `className`. The layer-only, API and "listed in full" rows, and the "ambient by clause 1 to 3" figure of 13 the proposal below relies on, have not been re-measured since, so quote them as of that commit.

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

`scripts/bench.py` builds both in release mode, then times `extract --json` over Mantine and Chakra in the Storybook addon's configuration (dependencies loaded, cache on after the warm-up run, inherited props not collapsed) over corpora with their dependencies installed with the two builds interleaved and alternating which goes first. It prints the median wall time and peak RSS of each and their ratio. It stops with the setup command to run when a corpus is not installed, because a sparse fetch has no dependencies to load: run `scripts/corpora.py setup --corpus mantine --corpus chakra` first.

The 1.10× and 1.20× thresholds come from a calibration made when the bench ran with `--no-cache --no-load-dependencies` (with nothing changed, the ratio stayed inside 1.04 for time, 95th percentile of 100 trials per corpus on CI and a laptop, and inside 1.12 for memory on CI runners). The bench now runs the addon's configuration (dependencies loaded, warm cache) and that calibration has not been repeated for it, so the noise band may differ. More than 1.10× in time or 1.20× in memory is a starting point for looking. Nothing enforces that. Run it before a resolver change goes up for review, and say in the pull request when a cost is intended.

## Scripts and their callers

Every `sh`, `py` and `mjs` script in the repository has a caller in a CI job, the weekly or a documented recipe. A measurement run once is recorded in this directory's documents, not committed as a script.

- `scripts/weekly.py`: the `weekly` workflow (`.github/workflows/weekly.yml`).
- `scripts/agent_eval.py` and `scripts/agent_questions.json`: `weekly.py --agent-eval`.
- `scripts/headline.py`: `weekly.py`.
- `scripts/corpora.py`: the `weekly` workflow (`setup`), `weekly.py`, `agent_eval.py` and `bench.py`.
- `scripts/oracle_data.py` and `scripts/schema_version.py`: `weekly.py`, `headline.py` and `agent_eval.py`. A Rust test (`crates/cli/src/commands/schema.rs`) holds `schema_version.py` to the version Rust writes.
- `scripts/rdt.mjs` and `scripts/rdg.mjs`: `weekly.py`.
- `scripts/bench.py`: `just bench`, the local check above.
- `scripts/gate_fixtures.mjs`: `just gate-measure`, the memory, determinism, first-answer and watch gates of ADR 0026 (`docs/gate-measurements.md`).
- `scripts/verify-install.sh`: `just verify-install`, the `Install` CI job.
- `scripts/check-packaging.mjs`: `just lint`, the `Lint` CI job. `scripts/napi-loader-version.mjs`: `build:napi` in `packages/napi`.
- `scripts/test_*.py`: `just test-py`, the `Test / Python scripts` CI job.

## Checker modes

The TypeScript 7.1 checker modes, then opt-in, were measured once, by hand, against the structural default of the time on the corpora `scripts/corpora.py` sets up. The measurement is not repeated and is not part of the weekly record; the results below are its record.

Per corpus it ran the structural default (`extract --format rdt`), the same with `--experimental-semantic-types` (the CLI's whole-project checker pass, the default since S8), and one lazy session driven the way the Storybook addon drives it (`createSession`, `initializeSessionIndex`, then `sessionComponentAnswers` per component) with the checker off and with `semanticTypes: 'on-demand'` at a 150 ms and a 600 ms wait. It also ran the CLI and a session against a missing `tsc` and one that exits at once.

The settings are the Storybook addon's (`createDocgenProvider` in `packages/storybook/src/provider.ts`): dependency types loaded, `collapseInheritedProps: 'none'`, a lazy session, and the NAPI defaults for the rest (`htmlAttributes: 'full'`, the on-disk cache on). The CLI runs passed the same props settings (`--load-dependencies --collapse-inherited-props none --html-attributes full`) so that the structural and the checker run list the same props. The sessions left out the per-story look at the files (`ensureFresh`, a few milliseconds on a large project) and `warmUp`/`fillAfter`, both off in the addon by default.

How to read the figures:

- Wall time and peak RSS are the median over 3 runs. Peak RSS is the largest single process, so a CLI checker run reports the larger of Caliper and `tsc`. A session's whole-process figures mean something only with the checker off: an on-demand run sleeps before asking the same components a second time, so its own step timings are reported instead. The checker's size comes from polling `ps` from the parent while the session runs, never from inside the session. The checker is the direct child of the session driver that is a `tsc` in `--api` mode, found by what it is and not by a string in a command line: the driver's and the measuring helper's own command lines carry the `tsc` path in their JSON options and can use more memory than the checker, and the configured `bin/tsc` is a launcher for the platform binary it resolves to.
- First load for a session is one request at a time in a seeded order that is the same in every mode: start (create plus index), the first component, and the first 10 and 50 components asked. The first run started with an empty cache directory (cold), the others with the cache it left (warm).
- For the on-demand mode: the share of first requests the checker answered within the wait, how many timed out, and the share answered once the late answers had arrived.
- The props that differ from the structural result are type, required, default and presence. `@types/react` props are counted apart (the whole-project checker pass leaves them out), and a type that differs only in spelling (spacing, `| undefined`, union order, `Array<T>`) is counted as cosmetic. Each remaining difference has a coarse category: `only_checker`, `only_structural`, `parameter_marker_lost`, `checker_longer` or `checker_shorter` (the checker's text is more than 20 characters and more than 1.5 times longer or shorter, a length test that says nothing about why, so a longer text may be an alias written out, an object type or a function type), and `other_type`.
- The checker oracle's verdict on each mode is for information only: the shares of shared props it calls identical or spelled differently (agree), less precise, and wrong. The oracle is the same checker, so it agrees with a checker mode almost by construction, and it credits the structural result for keeping a type's alias name while it scores the checker's expanded union as less precise (`docs/type-checker-integration.md`). Do not read that agree figure as accuracy; judging the checker modes needs a reference that is not the checker. The oracle step ran the CLI with `--html-attributes full --include-unexported --load-dependencies` and the default collapse; the weekly record's own oracle step collapses nothing (`--collapse-inherited-props none`).

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

## Collapse by relevance, 2026-11-02

A dependency source used to collapse into a layer of names once it declared more than 30 props. It now collapses when it is ambient (DOM and ARIA attribute families, style systems; clauses in `crates/core/src/resolver/ambient.rs`) and past 30 props, or when it is not ambient but past 200 props (`LARGE_API_SOURCE`). Every other dependency source is listed with its types. The rule is checked by a fixture of the verdicts of the oracle's classifier (`crates/core/src/resolver/ambient_parity.json`); that classifier's write-up is "Layer-only props" above.

**Who it changes.** The Storybook addon runs with `collapseInheritedProps: none` (`packages/storybook/src/provider.ts`), which lists every inherited prop, so the rule does not change what the addon shows. The output is byte-identical between the two builds on all five corpora in that mode. The rule changes the CLI and NAPI default, and so what agents and TOON readers see.

It was measured once, by hand, and the script is not kept: two builds (`10f1f072`, the base, and this branch), five corpora on the pinned commits, five interleaved runs, `--no-cache`, whole-process wall time and peak RSS from `/usr/bin/time -l`, load average 29 at the start, so read differences under about 20% as noise: the `none` runs, which execute the same code and print identical output in both builds, differ by up to 16% in a lazy session.

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
