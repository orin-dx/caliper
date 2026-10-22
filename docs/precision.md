# Detection precision

How often a component Caliper reports really is one, per detection rule (`detectedBy` in the output). The roadmap's gate is 95% per rule before a rule's detections count toward recall or before the rule is extended.

## Method

`scripts/precision-sample.py` runs `extract` on each corpus, groups components by `detectedBy`, and draws up to 25 per rule with a fixed seed. Each item was labelled `component`, `not_component` or `unsure` from its source: a component is something a consumer can render as `<Name />`.

The 2026-10-21 sample was labelled by four model agents reading each item's snippet and, where needed, the file. Those labels are not yet checked by a human.

## Results, 2026-10-21

Corpora: the five source-only mirrors (Mantine, chakra-ui, Ant Design, shadcn/ui, react-spectrum), on `main` as of #90. #91's stricter render evidence removes one more detection, antd's `QRcodeStatus`, which is a component.

| Rule                      | Detected | Sampled | Not a component | Precision |
| ------------------------- | -------: | ------: | --------------: | --------: |
| `functionDeclaration`     |    2,985 |      25 |               1 |       96% |
| `wrapperTypeArgument`     |      701 |      25 |               0 |      100% |
| `hocWrapper`              |      367 |      25 |               0 |      100% |
| `forwardRef`              |      185 |      25 |               0 |      100% |
| `fcAnnotation`            |      119 |      25 |               0 |      100% |
| `typedFunctionExpression` |      106 |      25 |               0 |      100% |
| `classComponent`          |        4 |       4 |               0 |      100% |

The miss: react-spectrum's `ListBoxDropIndicatorWrapper` is a `(props, ref)` render function handed to `DropIndicatorContext` and called, not rendered.

## What this does and does not show

- No rule shows a precision problem: 1 miss in 154.
- 25 samples cannot establish the 95% gate. With no misses in 25, the 95% lower bound is about 88%; showing 95% needs about 60 clean samples per rule (rule of three). The two inferred rules, `typedFunctionExpression` and `wrapperTypeArgument`, are the ones to grow first.
- Precision only: this says nothing about components Caliper misses (recall) or about whether their props are right.
