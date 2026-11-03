# Migrating from react-docgen-typescript

`caliper-docgen` targets output-shape compatibility with `react-docgen-typescript` (RDT) via the CLI's `--format rdt` flag. The NAPI binding and Vite plugin's virtual module expose this tool's own canonical format (`kind`-tagged `PropType`, shown in the main README). For RDT's shape at that layer, a NAPI session returns each component's `__docgenInfo`, and the Vite plugin's `injectDocgenInfo` option assigns it.

This document covers what's compatible today, what's intentionally different, and what's a known gap rather than a design choice.

## Should you switch today?

Output should match closely if your components use:

- Plain interfaces or type aliases
- `forwardRef` (including wrapped in an `as` cast), `FC`, or HOC wrapping
- `Omit` / `Pick`
- `VariantProps` (CVA / TV / PandaCSS)
- Discriminated unions (including ones wrapped in an intersection)

The full 250+ inherited HTML attributes per component are exposed by default; `--html-attributes curated` shortens that — see [HTML attribute inheritance](#html-attribute-inheritance) below.

## Field-by-field comparison

### Component level (`ComponentDoc`)

| RDT field | `--format rdt` | Notes |
| --- | --- | --- |
| `displayName` | ✅ |  |
| `description` | ✅ | From the component's own leading JSDoc, if any |
| `props` | ✅ | See prop-level table below |
| `methods` | ✅ (always `[]`) | RDT only populates this for class components; we don't support class components, so it's always empty — present for shape compatibility, not because we detect methods |
| `tags` | ✅ | JSDoc `@tag` values on the component declaration (e.g. `@deprecated`, `@since`) — not part of RDT's original spec but included since some tooling reads it |

### Prop level (`PropItem`)

| RDT field | `--format rdt` | Notes |
| --- | --- | --- |
| `name` | ✅ |  |
| `required` | ✅ |  |
| `type.name` | ✅ | Literal unions emit `{name: "enum", value: [...]}` matching RDT's convention (so Storybook-style `<select>` controls activate) instead of inlining the literal text |
| `description` | ✅ |  |
| `defaultValue` | ✅ | `{value, computed}` — captured from destructured parameter defaults (`{ variant = 'primary' }`) or JSDoc `@default`, code value wins on conflict |
| `parent.name` / `parent.fileName` | ✅ | `fileName` is always canonicalized to an absolute path regardless of how `--src` was invoked |
| `props.children` | ✅ | Omitted when it has no description, matching RDT's default `skipChildrenPropWithoutDoc`; `--format canonical` keeps it |
| `declarations` | ❌ not emitted in `--format rdt` | Only present in `--format canonical` (the raw internal shape); omitted from `--format rdt` to match RDT's single-parent shape. File an issue if you need it |

### Intentionally omitted

| Prop | Why |
| --- | --- |
| `key` | A React internal, not a prop a component receives. RDT includes it because it walks the full inherited type; we treat it as noise |

### HTML attribute inheritance

RDT inlines every inherited HTML attribute (e.g. all ~250-300 members of `ButtonHTMLAttributes`) directly into `props`. This tool does the same by default (`full`); set `--html-attributes` to shorten or remove them:

| Mode | Behavior | Reaches `--format rdt` |
| --- | --- | --- |
| `curated` | A compact subset per element (handlers, `aria-*`, `ref`, common attributes, the element's notable ones) in `notableInherited`. `inheritance` (canonical format only) still records the full layer — element, `Omit`-ted keys. | No |
| `full` (default) | Resolves `@types/react`'s real `HTMLAttributes`/`AriaAttributes`/`DOMAttributes`/`<Element>HTMLAttributes` chain and merges it directly into `props`, matching RDT's flat behavior. | Yes |
| `none` | Own-declared props only. | Yes (nothing to add) |

Flag by layer: `--html-attributes full` (CLI), `htmlAttributes: 'full'` (NAPI), `htmlAttributes: "full"` (`docgen.config.ts`).

**`full` mode accuracy:** verified against a real button, 238 of ~250 real attributes resolve. The remainder is a narrow gap — a handful of `@types/react` fields reference a same-namespace sibling type without an explicit qualifier, which degrades to an `UNRESOLVABLE_IMPORT` diagnostic on that one field rather than a crash or missing component. Details in `docs/rdt-coverage.md`'s "HtmlAttributeMode" section.

If you set `curated` or `none`, the attribute set is smaller than RDT's; that is the main compatibility gap to check for your specific components.

## Getting the exact diff for your codebase

`scripts/weekly.py` compares this tool's output against a real `react-docgen-typescript` run, prop by prop, on five pinned public libraries (Mantine, Chakra, shadcn, Ant Design, React Spectrum), and `docs/measurement.md` explains how to read it. To see the difference on your own components, run `caliper-docgen extract --src <dir> --format rdt` and compare its props with your current RDT output.

`docs/rdt-coverage.md` tracks which pattern each fixture exercises and the bugs found along the way (almost all now fixed).
