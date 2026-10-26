# Confidence routing experiment

The score uses only Caliper's emitted prop shapes, component metadata, and diagnostics. TypeScript 7.1 supplies offline labels for evaluation. A production score would not require the TypeScript 7.1 opt-in; invoking the semantic backend for a routed component would still be an explicit product decision.

## Result on the pinned corpora

Measured on 2026-10-25 with the structural binary at `3f20f965` and its PR stack, `typescript@7.1.0-beta`, and the five corpora in `scripts/parity.py` plus fixtures. The checker and Caliper share 518,297 labelled props. The oracle marks one prop `wrong`: `RouterProvider.navigate` in Spectrum. Its source preserves `Href` and `RouterOptions` aliases; the checker instantiates the unaugmented `RouterConfig` to `string` and `undefined`. This is a spelling and environment difference, not evidence of a consumer-visible incorrect function signature. One hard label cannot validate an automatic fallback rule.

| Target at 10% of matched components routed | Existing warning rule | Smoothed table | Logistic model |
| --- | ---: | ---: | ---: |
| Shared `wrong` or `less precise` props, five-corpus mean | 10% | 27% | 25% |
| Checker-only props outside summarized layers, five-corpus mean | 24% | 3% | 8% |

The soft label includes many type spellings that may be useful to preserve in RDT-compatible output. The table and model catch no hard `wrong` labels at a 20% component budget. Routing individual props has the same missing-prop blind spot: a prop that Caliper never emitted has no prop-level score.

## Missing props

The checker reports 1,182 props absent from matched shadcn components after excluding props attributed to intentionally summarized inheritance layers. Of these, 1,154 originate in Next's ambient augmentation, 22 in the TypeScript DOM library, and 6 in Radix Menubar. The 22 DOM props belong to an exported `GET(request: Request)` route handler that Caliper mistakenly classified as a component because its `ImageResponse` constructor receives JSX; they are a false component match, addressed after this measurement in #209. The other corpora have 27 antd, 12 Chakra, 67 Mantine, 52 Spectrum, and 13 fixture missing props by the same heuristic. The matching procedure does not evaluate components the checker finds but Caliper does not match.

These are separate coverage labels, not `wrong` type labels. The layer exclusion matches declaration paths, so it can misclassify a prop when different types share one file. A routing policy trained on these labels would mostly learn package augmentation and corpus composition rather than structural extraction risk.

## Reproduce

After `scripts/parity.py setup` has installed the pinned corpora and `cargo build --release` has built the CLI:

```bash
python3 scripts/parity.py oracle --bin target/release/caliper-docgen --corpora .parity/corpora --work .parity/confidence-oracle
python3 scripts/confidence.py --work .parity/confidence-oracle --out .parity/confidence-report.md
```

`oracle` writes every shared-prop verdict, the checker-only and summarized prop rows, and their declaration paths. `confidence.py` evaluates a warning rule, a smoothed table, and a logistic model with leave-one-corpus-out fitting and equal corpus weight. The script reports both component and prop budgets, and keeps the one hard `wrong` label visible.

No automatic checker routing follows from this experiment. Continue fixing reachable structural gaps and measuring missing props separately from shared-prop type disagreements.
