# Performance gates

`scripts/scorecard.py` measures Caliper on five public corpora at pinned commits, and `.github/workflows/scorecard.yml` fails when it regresses. The criterion benches (`cargo bench -p caliper-docgen-core`) and `docs/benchmarks.md` cover single fixtures; this covers whole libraries.

## Corpora

Each corpus is a shallow, sparse fetch of one directory at a pinned commit, plus the files above it (root `tsconfig.json`, `package.json`). No dependencies are installed, so imports from other packages stay unresolved; that is the "Unresolved" column.

| Corpus   | Repository              | Commit     | Source directory             |
| -------- | ----------------------- | ---------- | ---------------------------- |
| mantine  | `mantinedev/mantine`    | `194d6524` | `packages/@mantine/core/src` |
| chakra   | `chakra-ui/chakra-ui`   | `960ae222` | `packages/react/src`         |
| shadcn   | `shadcn-ui/ui`          | `3d98900e` | `apps/v4`                    |
| antd     | `ant-design/ant-design` | `f8f5595a` | `components`                 |
| spectrum | `adobe/react-spectrum`  | `d5fb723a` | `packages`                   |

Changing a pin or a directory changes every figure, so do it in its own PR with the new table.

## What is recorded

Per corpus:

| Field | Meaning |
| --- | --- |
| kLOC | Lines in the `.ts`/`.tsx` files discovery includes |
| Components, props | From `extract --json`; props are summed over components |
| Unresolved | Warning and error diagnostics with an unresolved-type code (`UNRESOLVABLE_IMPORT`, `OPAQUE_TYPE`, `INDEXED_ACCESS_OPAQUE`, `TEMPLATE_LITERAL_OPAQUE`, `GENERIC_ARGUMENT_MISMATCH`, `UNKNOWN`) |
| Wall ms | Median wall time of `extract --json` with a warm parse cache: process start to exit, output included |
| Extract ms | Median of the run's own `stats.durationMs` |
| Peak RSS | Largest peak RSS over the runs |
| RDT ms | react-docgen-typescript parsing the same files as one program (`scripts/scorecard-rdt.mjs`), in-process, Node startup excluded |

## Gates

`--check` fails the run when any of these holds:

| Gate | Fails when | Why |
| --- | --- | --- |
| Regression | Wall ms is more than 1.1× the base build's, on any corpus | The base (main) is built and timed in the same job, runs interleaved, so the runner's speed cancels out |
| RDT ratio | Caliper is less than 10× faster than RDT, on any corpus | The point of the tool. Caliper's time includes process startup and RDT's does not, so the ratio is a lower bound |
| Scaling | Two copies of mantine take more than 2.2× as long to extract as one | Doubling files, components and props must not more than double the work; name collisions between the copies exercise disambiguation |

RDT takes 1 to 2 s per corpus here, so CI times it live instead of reading a committed baseline. `--rdt-baseline FILE` reads RDT times from an earlier scorecard JSON for machines without Node.

## Size model

Extraction time fits t ≈ 6.4 ms + 0.13 ms/kLOC + 3.8 ms/kprop (props as counted above). It is a sizing guide, reported as "Model ms", not a gate: a fixed cost that varies by machine dominates small corpora, so per-corpus residuals vary too much to gate on. On the run below the model is within 2× of the measured extract time on every corpus (0.45× for mantine to 1.6× for chakra).

## Results, 2026-10-22

Apple Silicon laptop, macOS, load average 8 to 12, 10 runs per corpus, branch `ci/size-normalized-gates`.

| Corpus | kLOC | Components | Props | Unresolved | Wall ms | Extract ms | Peak RSS MB | Model ms | RDT ms | vs RDT |
| --- | --: | --: | --: | --: | --: | --: | --: | --: | --: | --: |
| mantine | 79.6 | 355 | 19,763 | 146 | 63.6 | 41.5 | 153 | 92 | 1,263 | 20× |
| chakra | 40.8 | 600 | 560 | 403 | 33.6 | 23.0 | 66 | 14 | 1,321 | 39× |
| shadcn | 489.6 | 2,140 | 1,615 | 1,526 | 80.1 | 64.0 | 104 | 76 | 1,906 | 24× |
| antd | 172.4 | 365 | 3,514 | 316 | 61.3 | 48.0 | 87 | 42 | 1,014 | 17× |
| spectrum | 256.6 | 1,482 | 7,031 | 1,901 | 80.9 | 60.0 | 116 | 66 | 1,787 | 22× |

Scaling: one copy of mantine 26 ms, two copies 46 ms (1.77×).

RDT's component and prop counts are not comparable with Caliper's here: without dependencies it reports every exported function and expands what it can of `@types/react`. Recall and precision against RDT are in `docs/precision.md` and `docs/projects/architecture-roadmap.md`.

## Dependency loading against RDT, 2026-10-23

Whether `--load-dependencies` can become the default (ADR 0010). The five repos with their dependencies installed, RDT run on the same component files through each repo's sources (`skipLibCheck`, dependencies resolvable), Caliper in RDT's format with `--html-attributes full`. The 20 fixture libraries can't answer this: they are single copied files with no installed packages.

| Corpus | RDT props Caliper reports, off → on | Unresolved warnings, off → on | Shared props whose type text changed: agree → disagree / disagree → agree | Wall ms, off → on | Peak RSS MiB, off → on |
| --- | --- | --- | --- | --- | --- |
| mantine | 21.8% → 21.8% | 16 → 12 | 1 / 0 | 52 → 56 | 105 → 108 |
| chakra | 0.1% → 62.4% | 325 → 27 | 0 / 0 | 43 → 497 | 82 → 1,341 |
| shadcn | 0.2% → 4.5% | 829 → 265 | 6 / 2 | 67 → 104 | 104 → 176 |
| antd | 59.2% → 72.3% | 116 → 53 | 4 / 1 | 61 → 70 | 110 → 126 |
| spectrum | 97.1% → 97.1% | 174 → 166 | 0 / 0 | 116 → 121 | 191 → 215 |

- **Accuracy passes.** Coverage of RDT's props rises and unresolved names fall; of about 59,000 props both modes report, 11 stop matching RDT's text. All 11 are a named object or union type from a package that now prints expanded (`{ type: "file"; … }` for `FileUIPart`), which is how a project-declared object alias already prints (#104); RDT prints the name.
- **Chakra's cost blocks the default.** Loaded dependencies let each of Chakra's 600 components reach its own ~705-prop style system: 398,000 props, 497 ms and 1.3 GB (RDT: 637,000 props, 27 s, 2.7 GB). `--collapse-inherited-props all` cuts that to 5,515 props but still 281 ms and 835 MiB, so most of the cost is not the listed props.

RDT (`withCustomConfig`, `shouldExtractLiteralValuesFromEnum`, `shouldRemoveUndefinedFromOptional`) took 1.6 to 27 s and 0.6 to 2.7 GB per repo. The fixture harness also gained the switch: `CALIPER_LOAD_DEPENDENCIES=1 pnpm --filter caliper-docgen-validate run:ours`, then `parity.ts caliper-docgen`.

## Running it

```bash
cargo build --release -p caliper-docgen
pnpm install                                   # for --rdt
python3 scripts/scorecard.py --bin target/release/caliper-docgen --rdt --out scorecard.json
python3 scripts/scorecard.py --bin NEW --base-bin OLD --rdt --check   # the CI gates
```

Corpora are cached in `.scorecard/corpora` (gitignored). In CI, run the workflow by hand on a branch to gate it against main, or pass `base` to compare with another ref.
