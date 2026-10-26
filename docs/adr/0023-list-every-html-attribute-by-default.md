# 0023. List every HTML attribute React declares by default

**Status:** Accepted **Date:** 2026-10-26 **Supersedes:** the "curated by default" clause of [0013](0013-curated-mode-reads-react-attributes-apart.md) and the `htmlAttributes` default recorded in `docs/rdt-coverage.md`

## Context

`htmlAttributes: 'curated'` was the default. It was meant to keep the props table short, but it left props React declares out of the answer: `--format rdt` and the addon's `docgenInfo` never emitted `notableInherited` at all, so no handler of a curated component reached Storybook. A props type that reached an element through `Omit<HTMLAttributes<…>>`, `ComponentProps<"div">` or `PropsWithoutRef<…>` had no element layer in `full` either (antd's `PurePanel`, `Dragger`'s `capture`, `ref` on `forwardRef` components).

The Storybook addon's `collapseInheritedProps` already hides props inherited from dependencies, so a long list in the output does not make the UI long.

## Decision

`PipelineOptions::html_attributes` defaults to `full`: the CLI, `docgen.config.ts`, the NAPI binding, the Vite plugin and the Storybook addon all follow, and `skipHtmlProps` still means `none`. `curated` stays as an opt-in compact mode: `ref`, `aria-*`, every event handler but the `*Capture` twins, the global attributes and the element's notable ones, read from React's declarations and listed beside `props` in `--format rdt`. `key` is omitted in every mode; `ref` is listed for `ComponentProps<"div">` props and for `forwardRef` components.

Without `@types/react`, `full` lists no inherited attributes and reports one info diagnostic per component that needed them.

## Consequences

Measured on the five parity corpora with dependencies loaded (the default), canonical JSON, no cache, median of 11 (wall) and 21 (CPU, peak RSS), old default (`curated` at 338d1ddc) → new default:

| Corpus   |  Wall ms |  CPU s | Peak RSS MiB | JSON MB |
| -------- | -------: | -----: | -----------: | ------: |
| antd     |  56 → 62 | 0.33 → 0.37 | 130 → 151 | 3.6 → 6.3 |
| chakra   | 424 → 470 | 3.13 → 3.72 | 514 → 577 | 11.1 → 16.7 |
| mantine  | 149 → 164 | 1.05 → 1.21 | 175 → 229 | 8.4 → 14.2 |
| spectrum |  90 → 98 | 0.68 → 0.73 | 171 → 191 | 8.3 → 11.7 |
| shadcn   | 144 → 188 | 1.28 → 1.72 | 220 → 301 | 5.9 → 12.2 |

- **The default is slower and larger than the old one**: wall time +9 to +31%, peak RSS +12 to +37%, canonical JSON +42 to +105%. It is cheaper than the widened `curated` only on Chakra and shadcn (JSON 19.2 → 16.7 and 17.0 → 12.2 MB, CPU −8% and −5%).
- **Storybook parity** (`props Storybook lists that the addon lacks`, antd / shadcn / Spectrum sampled components): 37 / 0 / 1 → 34 / 0 / 1. `Dragger`'s `capture` and `Transfer`'s `ref` are listed; `ref` on `Breadcrumb` and Spectrum's `TreeView` (not detected as `forwardRef`) and 33 props on `PurePanel` remain. The 33 are Drawer's own props, which the declared `PurePanel` type does not extend.
- The RDT-shaped corpus snapshots grow from 0.35 MB (old curated) to 2.9 MB (widened curated) and 6.4 MB (full).
