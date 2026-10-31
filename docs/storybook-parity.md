# Storybook parity

`packages/storybook/scripts/storybook-parity.mjs` measures the Storybook addon against Storybook's own docgen on real repositories: how often the addon hands a component on, what it documents that Storybook doesn't, where the two disagree, and what each costs. It is not run in CI; it needs the corpora `scripts/parity.py setup` fetches, installed, plus the native binding.

```bash
pnpm --filter caliper-docgen-storybook build
node packages/storybook/scripts/storybook-parity.mjs --corpora .corpora/repos --sample 120 --out report/storybook-parity
```

Output: `<out>.json` (every number) and `<out>.md` (tables). Storybook writes its cache into each corpus's `node_modules/.cache`, so the run needs write access there.

## What is compared

| Side | How it runs |
| --- | --- |
| Storybook's docgen | `@storybook/react`'s own `docgen-worker.js` (react-component-meta), unmodified, called as a middleware. It needs the TypeScript 5/6 compiler API, so `typescript` is redirected to `apps/oracle`'s TypeScript 6 (`--typescript` overrides); this package's TypeScript 7 has none. |
| The addon | `dist/provider.js`, the real `createDocgenProvider`, with Storybook's `meta.component` resolver and argTypes converter. |

Variants of the addon:

| Variant                | `loadDependencies` | `collapseInheritedProps` | `fallback`                           |
| ---------------------- | ------------------ | ------------------------ | ------------------------------------ |
| `caliper`              | on                 | none                     | degraded (the addon's defaults)      |
| `caliper-nodeps`       | off                | none                     | degraded (what the addon did before) |
| `caliper-curated`      | on                 | dependencies             | degraded                             |
| `caliper-never`        | on                 | none                     | never                                |
| `caliper-nodeps-never` | off                | none                     | never                                |

The `-never` variants answer for every component whatever its diagnostics say. Comparing them shows the information the addon holds; comparing the others shows what the `degraded` rule lets through.

Two story sets per corpus:

- **Real stories**: the corpus's own `*.stories.*` files that name `component:` (Chakra: 1 of 125, Spectrum: 129 of 132; Mantine, shadcn and antd have none).
- **Sampled components**: one generated `export default { component }` story per sampled component (`--sample`), written beside the component and removed afterwards. Components that aren't a named or default export are skipped.

Per story, both sides' argTypes are reduced to control kind (`type.name`), options, required, type text, default and whether a description exists, then diffed prop by prop. **Extra props** are props only the addon lists. The report also has the session-level fallback rate: components whose extraction carries a warning or error, over all components, without any story.

Not measured: Storybook's control inference from argTypes (kind is `type.name`), and runtime behaviour in a real Storybook process (the worker thread, concurrency, caching). Timings include the first request, which starts the session; the TypeScript baseline's include building its program. Storybook with thousands of stories scales differently from these samples.

## Results

2026-10-24, main through #152 plus the addon options of #155, Apple silicon. Peak RSS is the child process's.

### Fallback rate: components the addon would hand on

| Corpus   | Components | nodeps | deps (default) | deps, curated |
| -------- | ---------- | ------ | -------------- | ------------- |
| Mantine  | 344        | 70.1%  | 68.6%          | 68.6%         |
| Chakra   | 609        | 55.9%  | **92.1%**      | 92.1%         |
| shadcn   | 1892       | 45.8%  | 42.1%          | 40.6%         |
| antd     | 248        | 42.3%  | 27.4%          | 27.0%         |
| Spectrum | 1647       | 38.4%  | 37.9%          | 37.9%         |

Session init: 47–91 ms without dependencies, 76–490 ms with; peak RSS 128–261 MB without, 157–786 MB with.

- **Loading dependencies lowers the rate except on Chakra**, where it rises from 56% to 92%: Chakra's style props pull csstype in, 557 components hit `Max resolution depth exceeded` and 561 get unresolved-reference warnings, and the session holds 470,957 props (773 per component) against 582 without dependencies. Without them those components answer with almost no props (the 34 sampled Chakra stories it answers miss 39,008 props Storybook lists), so the `degraded` rule is right to hand them on.
- `collapseInheritedProps: 'dependencies'` changes nothing on Chakra, Mantine and Spectrum and trims shadcn (13,244 → 7,682 props).
- The leading reasons are unresolved references to dependency types (Chakra, Spectrum, antd), `cannot be used as a component` and `Cannot find the component whose props are referenced` (shadcn).

### Against Storybook's docgen (sampled components; `caliper` = the defaults)

| Corpus (stories) | Storybook s / MB | caliper s / MB, answered | caliper-never s / MB | Extra props | Props only Storybook lists | Default only Storybook |
| --- | --- | --- | --- | --- | --- | --- |
| Mantine (18) | 1.0 / 462 | 0.2 / 252, 14 | 0.2 / 249 | 0 | 8 in 4 stories | 0 |
| Chakra (102) | 4.0 / 1186 | 0.8 / 787, **10** | 1.3 / 844 | 9 | 993 in 13 | 39 |
| shadcn (112) | 3.3 / 1218 | 0.8 / 323, 68 | 0.8 / 326 | 446 (87 described) | 76 in 21 | 21 |
| antd (83) | 2.8 / 1051 | 0.4 / 238, 43 | 0.5 / 239 | 41 | 129 in 13 | 39 |
| Spectrum (102) | 2.6 / 1384 | 2.3 / 331, 63 | 1.9 / 328 | 1683 (576 described) | 7 in 6 | 48 |
| Spectrum real stories (129) | 1.3 / 686 | 2.8 / 340, 22 | 2.2 / 352 | 6530 (2390 described) | 0 | 0 |

- **Extra information.** What the addon lists and Storybook's docgen doesn't is inherited library props: DOM event handlers (`onPointerMoveCapture`), `aria-*`, Spectrum's style props (`UNSAFE_className`, `marginY`, `zIndex`), `isDisabled`/`isInvalid` from react-aria hooks. 38% of Spectrum's real-story extras and 20% of shadcn's carry a description. Whether that is more help or more noise in a controls panel is a product choice (`htmlAttributes`, `collapseInheritedProps`).
- **Cost.** The addon is 1.2–5× faster on these samples and uses 1.1–4× less memory (Chakra with dependencies is the exception: 844 MB against 1186 MB). The TypeScript baseline's time grows with the programs' size; the addon's first request pays the whole-corpus session.
- **Where the addon is behind.** Defaults Storybook reports and the addon doesn't (39–48 per corpus), 129 props in 13 antd stories, 993 in 13 Chakra stories, control kind differing on 60 of 917 shared antd props, and type text identical on only 60–70% of shared props (formatting included).
- **Reachability.** 20 antd and 114 of 129 real Spectrum stories name a component outside the configured `srcDirs` or a path the session can't match, which counts as handed on. Spectrum needed `starters` added to `srcDirs`.

## Reading the next run

The numbers to watch after the session work: the `caliper` degraded rate per corpus (target: below the `-never` gap), `Props only Storybook lists` (target 0), and the `caliper` seconds and MB columns for the first request.
