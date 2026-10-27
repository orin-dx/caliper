# 0019. Load dependency declarations by default

**Status:** Accepted **Date:** 2026-10-25 **Supersedes:** the "off by default" clause of [0010](0010-load-dependency-declarations-by-name.md)

## Context

ADR 0010 shipped dependency loading off by default because of its cost: Chakra took 497 ms and 1.3 GB with it on, against 43 ms and 82 MiB without (`docs/performance.md`, 2026-10-23). `docs/STATUS.md` named that cost as the blocker. The Storybook addon already defaults `loadDependencies` to `true` (#155), so the CLI, the config file, the Vite plugin and the NAPI binding were the only surfaces where a prop typed by an installed package stayed a named reference with an `UNRESOLVABLE_IMPORT` warning.

The cost moved. Memo replay, `keyof` name walks, interned keys and a parallel lib parse (#136, #175, #177, #178, #182) took Chakra to about 220 ms and 330 MiB.

## Decision

`PipelineOptions::load_dependencies` defaults to `true`. `--no-load-dependencies` (CLI), `loadDependencies: false` (`docgen.config.ts`, NAPI `JsExtractOptions`, Vite plugin, Storybook addon) turn it off, and a flag overrides the config. `--load-dependencies` stays accepted and means the same as the default, so existing scripts keep working; it conflicts with `--no-load-dependencies`.

A project with no packages installed loads nothing, so there is nothing to opt out of there.

## Consequences

Measured on the five parity corpora, `extract --json`, best of 7, interleaved, on a laptop shared with other jobs (about ±10 ms), old default against new default:

| Corpus   |  Wall ms | Peak RSS MiB |           Props | Unresolved warnings |
| -------- | -------: | -----------: | --------------: | ------------------: |
| mantine  |  50 → 60 |    112 → 128 | 19,834 → 19,834 |             14 → 10 |
| chakra   | 50 → 220 |    101 → 330 |   592 → 479,235 |             315 → 0 |
| shadcn   |  50 → 90 |    101 → 225 |   1,543 → 8,131 |           816 → 190 |
| antd     |  40 → 50 |     95 → 114 |   3,349 → 3,856 |              76 → 8 |
| spectrum |  70 → 90 |    136 → 161 | 32,585 → 32,587 |              13 → 5 |

- **Output is exact.** On all five corpora the new default is byte-identical to the previous build's `--load-dependencies` output, and `--no-load-dependencies` is byte-identical to the previous default (canonical JSON without `durationMs` and `stats`).
- **Accuracy against react-docgen-typescript.** Share of its props reported, off → on (`scripts/parity-baseline.json`, 0.4.0, not re-measured here): Chakra 0.1% → 98.6%, shadcn 6.4% → 39.5%, antd 71.8% → 80.3%, Mantine and Spectrum 99.7% both.
- **The cost lands on a few projects.** Chakra is 4.4× slower and holds 3.3× the memory by default; shadcn takes 40 ms and 124 MiB more. Mantine, antd and Spectrum take 10 to 20 ms and 16 to 25 MiB more. A CI job that needs the old speed passes `--no-load-dependencies`.
- **No packages installed.** antd's `components` copied without `node_modules`: 3,349 props and 163 unresolved warnings either way, 40 ms with loading on against 30 ms off. One diagnostic's help text differs, because it says dependency types were loaded but a module could not be found.
- **Tools that pinned the old default** (`scripts/parity.py`, `apps/validate/src/run-ours.ts`, the corpus snapshot tests) now pin it explicitly, so their baselines keep their meaning.
- **A changed default, not a changed API.** `PipelineOptions` keeps its shape; the changeset is a minor bump.

## Alternatives considered

- **Keep it off and rely on the addon.** The CLI and plugin would keep reporting hundreds of unresolved names on installed projects (816 on shadcn) while the addon did not.
- **Turn it on only when a `node_modules` directory is found.** That is what loading already does: with no packages, the layer is empty. A second switch would add a mode with the same effect.
- **Flip the plugin and addon only.** It leaves the CLI and the config file disagreeing with the tools built on them.
