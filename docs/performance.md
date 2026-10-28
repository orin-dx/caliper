# Performance

How fast Caliper is and what it costs is recorded in `docs/measurement.md`: a weekly run on five pinned corpora, and `just bench` for a change that touches the resolver. The criterion benches (`cargo bench -p caliper-docgen-core`) and `docs/benchmarks.md` cover single fixtures. This page keeps the measurements that explain where the time goes.

The per-pull-request scorecard that used to enforce these numbers is gone. It caught one real regression (Mantine 38% and Chakra 47% slower after a batch of 20 resolver pull requests merged on 2026-10-26, found by the weekly run), and its gates failed on runner noise and on bases that had moved. The figures below came from it and are history, not limits.

## Size model

Extraction time fits t ≈ 6.4 ms + 0.13 ms/kLOC + 3.8 ms/kprop (props summed over components). It is a sizing guide, reported by the old scorecard as "Model ms", never a gate: a fixed cost that varies by machine dominates small corpora, so per-corpus residuals vary too much to gate on. On the run below the model is within 2× of the measured extract time on every corpus (0.45× for mantine to 1.6× for chakra).

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

RDT's component and prop counts are not comparable with Caliper's here: without dependencies it reports every exported function and expands what it can of `@types/react`. Precision is in `docs/precision.md`; recall and agreement with RDT on installed repos, gated weekly, are in `docs/parity.md`.

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
- **Fixed in #136: 497 ms and 1.3 GB became 255 ms and 0.3 GB.** Most of the cost was `keyof` walks resolving every member's type when only the names are read, and memo replays copying each recording's diagnostics into every component.
- **Chakra's peak memory with dependencies, by change** (`/usr/bin/time -l`, default threads): 367 MiB before the literal-type work, 494 MiB after member reads through csstype's `Fallback<T>` (#144), 388 MiB once an alias's arguments are resolved once and equal props are shared. Peak memory scales with threads (248 MiB on one thread, 388 on 18) because each thread holds the types it is building; measure memory before and after any change that evaluates more types, not only time.
- **Chakra's cost blocks the default.** Loaded dependencies let each of Chakra's 600 components reach its own ~705-prop style system: 398,000 props, 497 ms and 1.3 GB (RDT: 637,000 props, 27 s, 2.7 GB). `--collapse-inherited-props all` cuts that to 5,515 props but still 281 ms and 835 MiB, so most of the cost is not the listed props.

RDT (`withCustomConfig`, `shouldExtractLiteralValuesFromEnum`, `shouldRemoveUndefinedFromOptional`) took 1.6 to 27 s and 0.6 to 2.7 GB per repo. The fixture harness also gained the switch: `CALIPER_LOAD_DEPENDENCIES=1 pnpm --filter caliper-docgen-validate run:ours`, then `parity.ts caliper-docgen`.

## Storybook addon request path, 2026-10-24

Where the addon's time went, and what changed. Method: `packages/storybook/scripts/storybook-parity.mjs --no-baseline`, the `caliper-never` variant (it answers every component), before (`--dist` of the previous build and its binding) interleaved with after, best of 3, Apple silicon. Peak RSS is the child process's.

| Corpus, story set                 | Metric           | Before     | After      |
| --------------------------------- | ---------------- | ---------- | ---------- |
| Spectrum, 129 real stories        | total            | 2.17 s     | 0.63 s     |
|                                   | median per story | 15 ms      | 4 ms       |
|                                   | first request    | 200 ms     | 150 ms     |
|                                   | peak RSS         | 354 MB     | 322 MB     |
| Spectrum, 90 sampled              | total            | 1.67 s     | 0.42 s     |
| shadcn, sampled                   | total / median   | 641 / 5 ms | 335 / 2 ms |
| antd, sampled                     | total / median   | 421 / 4 ms | 273 / 2 ms |
| Mantine, sampled                  | total / median   | 164 / 6 ms | 134 / 2 ms |
| Chakra with dependencies, sampled | total            | 1.16 s     | 0.80 s     |
|                                   | first request    | 527 ms     | 342 ms     |
|                                   | peak RSS         | 828 MB     | 681 MB     |

What the profile found:

- **The per-request file sweep was the cost.** `fresh()` listed every directory and stat'ed every source file from JavaScript on each story request (16–22 ms on Spectrum), and again for each subcomponent. The session now does one native check per story (`ensureFresh`), stat calls in parallel: 3–5 ms on Spectrum. `look at the files` is still the largest per-story cost on a large project.
- **Session start parsed every prop.** `initializeSession` returned all components' props as JSON for JavaScript to read two fields from: 25 MB and about 120 ms of `JSON.parse` for Chakra with dependencies. `initializeSessionIndex` returns the file, name and diagnostics only.
- **Allocation.** `mimalloc` in the binding cuts session start by 15–30% (Chakra with dependencies 441–561 ms → 312–391 ms) and Chakra's peak by 9%. On small projects the peak is 20–30 MB higher.
- **Left to the resolver.** On Chakra with dependencies the time is in resolution: the largest self-time entries are `StateReads::append` (dependency read tracking), `ParsedProp` map building (773 props per component) and memcpy. Sampled with `sample(1)`; not changed here.
- **Not a cost:** `keyOf` and `degradedBy` (microseconds, now indexed), `docgenInfo` (0.05–0.1 ms per component, 2–3 ms on Chakra's 358 KB documents), the JavaScript side of a story (about 4 ms of CPU, the rest is waiting on native work).

## Lazy sessions, 2026-10-24

`WatchSession::lazy` (ADR 0016) against `WatchSession::new(..).initialize()` on the corpora above, with `load_dependencies` and full HTML attributes (the richest answer). `cargo run --release -p caliper-docgen-core --example lazy_bench` runs one scenario per process; 50 components are chosen by a fixed seed. Apple Silicon laptop, parse cache and dependency index warm, best of 5 interleaved runs (spectrum: best of 3 to 5 before merging main, so a range).

| Corpus | Components | Batch start | Lazy start | 1 component | 10 | 50, one at a time | 50 at once | Batch peak | Lazy peak after 50 |
| --- | --: | --: | --: | --: | --: | --: | --: | --: | --: |
| mantine | 344 | 68 ms | 40 ms | 52 ms | 67 ms | 81 ms | 68 ms | 108 MiB | 89 MiB |
| chakra | 609 | 352 ms | 45 ms | 172 ms | 249 ms | 566 ms | 310 ms | 532 MiB | 137 MiB |
| shadcn | 1,892 | 111 ms | 67 ms | 67 ms | 128 ms | 353 ms | 96 ms | 162 MiB | 134 MiB |
| antd | 248 | 71 ms | 60 ms | 66 ms | 114 ms | 213 ms | 141 ms | 119 MiB | 119 MiB |
| spectrum | 1,356 | 102 to 145 ms | 66 to 110 ms | 66 to 110 ms | 179 to 230 ms | 375 to 465 ms | not run | 135 to 178 MiB | 122 to 144 MiB |

Times are time from process start to the last answer (start plus requests); peak is the process's peak RSS (`/usr/bin/time -l`). Without dependencies and with curated attributes, lazy start is 28 to 85 ms against 39 to 107 ms for batch, peak memory is 50 to 114 MiB against 69 to 146 MiB, and 50 components cost 3 to 25 ms.

Every answer the example returned matched the batch run's for the same key (a hash of the full entries and diagnostics), serially and at once. `crates/core/tests/lazy_parity.rs` checks every component of the fixtures and of the five corpora, both settings, at once and one at a time in forward and reverse order.

The earlier lazy-session measurements did not cover Spectrum with both options on after merging main or the NAPI binding (the example calls the Rust API the binding wraps). The whole-project runs below cover more than 50 components.

## Chained lazy dependency layers, 2026-10-25

On the five installed parity corpora, `lazy_bench` with dependency loading and full HTML attributes, best of three runs per scenario. `random50` asks 50 seeded components one at a time; `all` asks every listed component in key order. Peak RSS is per process. The comparison uses the same build with the whole-demand threshold at three or 48 growths; this change sets the threshold to three (design in `docs/architecture/pipeline-orchestration.md`, "Demand-driven dependencies").

| Corpus   | 50 requests: 3 → 48 growths | Peak MB: 3 → 48 | All requests: 3 → 48 | Peak MB: 3 → 48 |
| -------- | --------------------------: | --------------: | -------------------: | --------------: |
| Mantine  |                  30 → 27 ms |         68 → 67 |         163 → 170 ms |         88 → 84 |
| Chakra   |                234 → 271 ms |       107 → 105 |     2,117 → 2,384 ms |       317 → 317 |
| shadcn   |                  52 → 72 ms |        144 → 94 |         213 → 272 ms |       159 → 159 |
| antd     |                  18 → 46 ms |         71 → 73 |          56 → 126 ms |         81 → 84 |
| Spectrum |                  20 → 47 ms |        102 → 90 |         162 → 222 ms |       134 → 134 |

All ten output digests match across thresholds and the pre-chain build. The three-growth chain is also faster than the pre-chain three-growth build on 50 requests for all five corpora: 42 → 30 ms on Mantine, 313 → 234 on Chakra, 61 → 52 on shadcn, 23 → 18 on antd, and 27 → 20 on Spectrum. A file first parsed as a re-export stub must be indexed when later claimed; without that step, 28 shadcn components lost props in the whole-project run. The regression test and forward-order corpus parity check cover it.
