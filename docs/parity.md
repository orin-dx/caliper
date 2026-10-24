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

Caliper runs in RDT's format with `--html-attributes full --include-unexported`, without and with `--load-dependencies`. On components both tools report (same file, same name):

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

## Running it

```bash
cargo build --release -p caliper-docgen
pnpm install                                     # RDT, from apps/validate
python3 scripts/parity.py setup                  # about 10 GB of installs into .parity/
python3 scripts/parity.py run --bin target/release/caliper-docgen --markdown parity.md
python3 scripts/parity.py run --bin NEW --base-bin OLD --baseline scripts/parity-baseline.json --check
```

`--corpus NAME` limits a run to some corpora, and `--no-timing` skips the timed runs. RDT's dump is made once per commit, in `.parity/rdt`.
