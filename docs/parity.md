# Parity with react-docgen-typescript

`scripts/parity.py` measures Caliper against react-docgen-typescript (RDT) on five public repos with their dependencies installed, and `.github/workflows/parity.yml` runs it weekly and by hand. The scorecard (`docs/performance.md`) times source-only corpora; this measures what the docs say, on code whose imports resolve.

## Corpora

Each repo is fetched at a pinned commit and installed with its own package manager and lockfile, with lifecycle scripts off and every cache under `.parity/` (gitignored). Tests, stories, demos and `.d.ts` files are then removed from the source directory, so both tools read the same files. RDT parses every `.tsx` file in it as one program, rooted at the source directory.

| Corpus | Repository | Commit | Source directory | Install |
| --- | --- | --- | --- | --- |
| mantine | `mantinedev/mantine` | `e47e45af06427b3b2497100c2b849935c23af30d` | `packages/@mantine/core/src` | yarn, immutable |
| chakra | `chakra-ui/chakra-ui` | `fa9696f328ea5a0df8685c6d8460edbd779b63b9` | `packages/react/src` | pnpm, frozen |
| shadcn | `shadcn-ui/ui` | `ba1503c64472b82256d9de95e2e62c5901a38e8b` | `apps/v4` | pnpm, frozen |
| antd | `ant-design/ant-design` | `ba7ccbaa7b4b58ac1ff8e6fe2e78dfa10bfb5538` | `components` | npm, no lockfile (the repo commits none) |
| spectrum | `adobe/react-spectrum` | `797762ed3526ea12f089e321bd3891066affad70` | `packages` | yarn, immutable |

A lockfile the pinned package manager rejects falls back to a fresh resolve; the install log says which ran.

## What is measured

Caliper runs in RDT's format with `--html-attributes full --include-unexported`, without (`--no-load-dependencies`, since dependency loading is on by default, ADR 0019) and with `--load-dependencies`; `scripts/parity.py` pins the flag for builds that know it. On components both tools report (same file, same name):

| Measure | Meaning |
| --- | --- |
| Components matched | RDT's components with props that Caliper also reports |
| Non-React props reported | RDT's props not declared by `@types/react` that Caliper reports. Caliper lists React's DOM attributes as one inherited layer, as Storybook shows them |
| Same text | Identical type text after RDT's own normalization (`\| undefined` dropped) |
| Same or cosmetic | Also identical after dropping quotes, nested `\| undefined`, namespace prefixes, `Array<T>` spelling, a trailing `;`, outer parentheses and union order |
| Same control | Both types imply the same Storybook control (boolean, number, text, select, function, node, object) |
| Loss patterns | Shared props whose type carries less than RDT's, by pattern: a member read left as `X["k"]`, a lost rest parameter, an unbound type parameter, and so on. Two patterns count the reverse, where RDT carries less |
| Time and memory | Best of N wall time and peak RSS of `extract --json`, without and with `--load-dependencies`, next to RDT's own parse time |

## Gates

`--check` against `scripts/parity-baseline.json` fails when any rate above falls by more than 0.05 points, any of Caliper's loss patterns grows, or, with `--base-bin`, the build is more than 1.1× slower than the base on any corpus in either mode. The base is timed interleaved in the same job, so the runner's speed cancels out; the baseline's own times are a record of one machine, not a gate. Regenerate the baseline with a release build of main when a change is meant to move these numbers, and say so in the pull request.

## Results, 2026-10-23

`main` at `7bc120f4`, Apple Silicon laptop, 7 runs. With `--load-dependencies`:

| Corpus | Components matched | Non-React props reported | Same text | Same or cosmetic | Same control | Caliper | RDT |
| --- | --: | --: | --: | --: | --: | --: | --: |
| mantine | 331/389 | 99.7% | 48.0% | 50.9% | 93.1% | 55 ms, 109 MiB | 1.3 s, 458 MiB |
| chakra | 592/773 | 98.6% | 19.4% | 19.4% | 100.0% | 550 ms, 1,461 MiB | 24.8 s, 1,550 MiB |
| shadcn | 1519/1594 | 39.5% | 61.3% | 74.7% | 86.7% | 115 ms, 181 MiB | 4.0 s, 1,149 MiB |
| antd | 162/258 | 80.3% | 69.6% | 78.5% | 94.8% | 86 ms, 127 MiB | 2.4 s, 1,033 MiB |
| spectrum | 1087/1408 | 99.7% | 67.4% | 68.9% | 96.9% | 130 ms, 219 MiB | 2.2 s, 828 MiB |

RDT's memory is its process's peak RSS; Caliper's time includes process start and RDT's excludes Node's. The full report, loss patterns included, is in `scripts/parity-baseline.json`.

## TypeScript's checker as the oracle

RDT is a second implementation with its own mistakes (on Spectrum it types `items` as its parent interface, `AriaMenuProps<T>`). `scripts/parity.py oracle` asks TypeScript 7.1's checker instead: `apps/validate/src/oracle.ts`, through the official `typescript/sync` client of `7.1.0-beta`, reads each prop's type for the components it finds the way RDT does, and compares it structurally with Caliper's canonical `--json` (with `--load-dependencies`). Both sides are reduced to kind, literal values, union members, object fields, function parameters (optional and rest included) and index signatures; the checker side also keeps the type's alias or declaring name. It is test-only: the semantic backend in `docs/type-checker-integration.md` is a separate, experimental opt-in.

Each prop React's `@types/react` doesn't declare gets one verdict:

| Verdict | Meaning |
| --- | --- |
| Identical | Both sides describe the same type |
| Spelled | The same type, Caliper by name where the checker expands it (`StyleProp<MantineSpacing>`, `ReactNode`) |
| Less precise | Caliper's type is looser or left unevaluated: an opaque type, `string` for a literal union, a name the checker resolves further, an unbound parameter, a lost optional or rest marker, a missing field or index signature |
| Wrong | Caliper's type describes something else, such as a literal union missing members or `number` where csstype's `ZIndex` also allows strings |
| Checker any/error | The checker has no type: an error, `any`, or a top-level `unknown` where Caliper has one |

A component's own type parameter that Caliper replaced with an instantiation (`multiple?: Multiple` with `Multiple extends boolean = false`, read as `false`) is "less precise" when Caliper's type is the parameter's default or its constraint, and "wrong" when it is neither; the checker side carries both. An optional prop of type `never` is the checker's `undefined`. `oracle.ts --dump FILE` (which `parity.py oracle` passes) writes every prop judged wrong or less precise, so two builds' verdicts can be diffed prop by prop; `apps/validate/src/oracle-compare.test.ts` holds the comparison's tests.

Checker types are normalized once per type id and verdicts made once per pair, so Chakra's 470,878 shared props cost 1,241 comparisons. `--check` against `scripts/oracle-baseline.json` fails when identical-or-spelled falls, wrong rises, or components matched falls, by more than 0.05 points. The fixtures directory is a sixth corpus, read in place.

`main` at `7bc120f4` against `7.1.0-beta`:

| Corpus | Components matched | Shared props | Identical | Spelled | Less precise | Wrong | Checker any/error | Checker time |
| --- | --: | --: | --: | --: | --: | --: | --: | --: |
| fixtures | 33/40 | 308 | 66.2% | 26.9% | 6.2% | 0.3% | 0.3% | 0.6 s |
| mantine | 332/370 | 19,752 | 16.7% | 73.3% | 9.6% | 0.3% | 0.0% | 1.2 s |
| chakra | 598/790 | 470,878 | 0.9% | 99.1% | 0.0% | 0.0% | 0.0% | 4.1 s |
| shadcn | 1531/1580 | 5,160 | 61.3% | 23.1% | 14.5% | 0.1% | 1.1% | 1.6 s |
| antd | 185/310 | 2,751 | 55.9% | 36.0% | 7.9% | 0.3% | 0.0% | 1.1 s |
| spectrum | 598/1454 | 11,663 | 20.2% | 47.6% | 30.2% | 0.0% | 2.0% | 1.6 s |

The checker finds more components than Caliper on antd and Spectrum because, like RDT, it counts every exported function with one parameter. "Less precise" on Spectrum is mostly `Responsive<DimensionValue>` printed without its object form. The beta needs two workarounds, both in `oracle.ts`: compiler options must come from `parseConfigFile` (its server panics on string-valued enum options passed to `createProgram`), and `true`/`false` literal types carry no `value`, so they are read with `typeToString`.

## Setting up the corpora

`parity.py setup` fetches each repo at its pinned commit and installs it with its own package manager (corepack shims at the versions each `packageManager` field pins). A corpus that fails to fetch or install is named on stderr with the tail of its install log (`.parity/logs/install-<name>.log`), as a GitHub annotation and in the job summary; the other corpora still install, and the command exits non-zero naming the failed ones.

`--install-node-dir DIR` runs only the corpora's installs with the `node` in `DIR`. The weekly workflow uses it to install under Node 24. That was not the fix for the `Dynamic require of "util" is not supported` that Mantine's and React Spectrum's installs hit under Yarn 4.18: the Node version is not the cause. Corepack keeps the package managers it downloads in `.parity/caches/corepack`, inside this repository, whose root `package.json` says `"type": "module"`. Node therefore loaded Yarn's CommonJS bundle `yarn.js` as an ES module, where `require` does not exist, on Node 24 and 26 alike. `parity.py setup` now writes a `{"type": "commonjs"}` `package.json` into the corepack home, so the nearest manifest above `yarn.js` is its own. Everything else in the job keeps the toolchain's Node.

## Equivalence with a base build

A change to how the resolver works (memos, interning, scheduling) should change speed, not output. `scripts/equivalence.py` checks that: it runs `extract --json` from a base build and from this checkout on the corpora, in four configurations (default, `--load-dependencies`, `--html-attributes full`, both; unexported components included), and compares the canonical forms. Canonical means the interned `props`, `propSets` and `nameLists` tables are expanded where they are used, and `durationMs` and the declaration-cache counters are dropped. Nothing else is ignored.

For each corpus and configuration it prints `identical`, or the differences by kind: `component-added`/`removed`, `prop-added`/`removed`, `prop-type-changed`, `prop-field-changed` (required, default, description, tags, parent, declarations), `notable-added`/`removed`/`changed` (the inherited attributes a component lists), `component-field-changed`, `diagnostic-added`/`removed`, `enum-changed`, `stat-changed`, with the first `--examples` of each. Any difference exits non-zero.

A change that is meant to alter output lists exactly what it alters in `scripts/equivalence-allow.json`, as fnmatch patterns over the printed `<corpus>/<config> <kind> <item>`, for example `shadcn/* prop-added Button*.size`. The file is empty on `main`; an entry belongs to the pull request that changes the output and goes with it. Two checks hold that: `equivalence.py` refuses an entry that names no component, prop or diagnostic (`antd/* prop-added *` would hide every prop added to antd, for every pull request after it), and the `Allow-list empty` workflow, with the same step in the weekly parity workflow, fails when `main` has any entry (`equivalence.py --require-empty-allow`). A stacked pull request compares with its own base, so it lists only what it changes beyond its parent.

Two further stages:

- `--lazy {off,fixtures,all}` runs `crates/core/tests/lazy_parity.rs`, the lazy session against batch extraction: the fixtures, and with `all` the selected corpora's sources (`CALIPER_PARITY_SRC`).
- `--oracle` compares the build with TypeScript 7.1's checker as `parity.py oracle` does and fails when agreement falls or the props the checker contradicts rise past `scripts/oracle-baseline.json`. It prints the wrong and less-precise rates next to the baseline's. A change that moves cycle or depth behavior needs this, because the snapshots cannot say whether the new output is closer to the checker.

Every stage has a time limit: `--timeout-build` (default 2400 s), `--timeout-extract` (600), `--timeout-lazy` (1800) and `--timeout-oracle` (3600). A stage that runs over is killed together with its child processes, reported by name with the tail of its output, and fails the run; the stages after it still run. The children get no stdin. A stage that waits for input or deadlocks can no longer stall CI silently. `python3 -I -B scripts/test_equivalence.py` tests the limits.

```bash
python3 scripts/parity.py setup --corpus mantine --corpus shadcn
just equivalence origin/main --corpus mantine --corpus shadcn --lazy fixtures    # a git ref is built once per commit
just equivalence ./caliper-before --bin target/release/caliper-docgen --oracle   # or an executable
```

`.github/workflows/equivalence.yml` runs it on pull requests that touch `crates/core/src/{resolver,extractor,types,pipeline}`: chakra and shadcn with dependencies loaded, against the pull request's own base (the two pnpm corpora; mantine and spectrum, which use yarn, can join now that their install works, see `--install-node-dir` above). It is non-blocking (`continue-on-error`) until it has run long enough to trust, and its time is not yet measured in CI; the corpus installs are the cost. The weekly parity workflow runs all five corpora and the lazy parity over their sources.

## Running it

```bash
cargo build --release -p caliper-docgen
pnpm install                                     # RDT, from apps/validate
python3 scripts/parity.py setup                  # about 10 GB of installs into .parity/
python3 scripts/parity.py run --bin target/release/caliper-docgen --markdown parity.md
python3 scripts/parity.py run --bin NEW --base-bin OLD --baseline scripts/parity-baseline.json --check
python3 scripts/parity.py oracle --bin NEW --baseline scripts/oracle-baseline.json --check
```

`--corpus NAME` limits a run to some corpora, and `--no-timing` skips the timed runs. RDT's dump is made once per commit, in `.parity/rdt`.
