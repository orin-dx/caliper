<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/hero-dark.png">
    <img src="assets/brand/hero-light.png" alt="caliper, an Orin DX tool" width="360">
  </picture>
</p>

<p align="center">
  <b>Fast React component prop extraction. Parses TypeScript with OXC, no type-checking pass.</b>
</p>

<p align="center">
  <a href="https://github.com/orin-dx/caliper/actions/workflows/ci.yml"><img src="https://github.com/orin-dx/caliper/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-FSL--1.1--MIT-blue.svg" alt="License" /></a>
</p>

---

`caliper-docgen` reads your components and reports each one's props: type, required or optional, default, and JSDoc. It is a faster alternative to `react-docgen-typescript` for Storybook, docs sites, and LLM tooling.

`react-docgen-typescript` builds a full TypeScript `Program`, which takes seconds on a mid-size design system. Caliper parses files in parallel with OXC and skips the type checker.

Cold run over 21 real component libraries, Apple M5 Max, October 2026:

|                                                    | Time  |
| -------------------------------------------------- | ----- |
| Caliper binary alone                               | 42ms  |
| Caliper through a Node harness                     | 0.38s |
| `react-docgen` through the same harness            | 0.52s |
| `react-docgen-typescript` through the same harness | 4.9s  |

`react-docgen` and `react-docgen-typescript` only run inside Node, so all three share one harness (`pnpm exec tsx`, 5 runs each). Starting that harness costs 0.32s on its own, which is most of the Caliper figure. A watch-mode update to one file takes about 2ms, where `react-docgen-typescript` pays the full cold cost on every edit. Method and raw numbers: [`docs/benchmarks.md`](docs/benchmarks.md).

## A component through Caliper

```tsx
interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Visual style. */
  variant?: 'default' | 'destructive' | 'outline'
  loading?: boolean
}

export function Button({ variant = 'default', loading, ...rest }: ButtonProps) {
  return <button data-variant={variant} aria-busy={loading} {...rest} />
}
```

```bash
caliper-docgen extract --src src --format rdt
```

| Prop      | Type                                      | Required | Default     | Description   |
| --------- | ----------------------------------------- | -------- | ----------- | ------------- |
| `variant` | `"default" \| "destructive" \| "outline"` | no       | `"default"` | Visual style. |
| `loading` | `boolean`                                 | no       |             |               |

By default only a curated subset of inherited HTML attributes is exposed; `--html-attributes full` exposes all of them (what `react-docgen-typescript` does). `--format canonical` is the tool's own shape, `rdt` matches `react-docgen-typescript`, and `toon` is a compact format for LLM agents.

```mermaid
%%{init: {'theme': 'base', 'flowchart': {'curve': 'basis', 'nodeSpacing': 40, 'rankSpacing': 56}, 'themeVariables': {'fontFamily': 'Inter, ui-sans-serif, system-ui, sans-serif', 'fontSize': '14px', 'lineColor': '#94a3b8', 'edgeLabelBackground': '#ffffff'}}}%%
flowchart LR
    classDef source fill:#eef2ff,stroke:#6366f1,stroke-width:1.5px,color:#1e1b4b,rx:10px,ry:10px,font-weight:600;
    classDef engine fill:#f5f3ff,stroke:#8b5cf6,stroke-width:1.5px,color:#4c1d95,rx:10px,ry:10px;
    classDef output fill:#ecfdf5,stroke:#10b981,stroke-width:1.5px,color:#064e3b,rx:10px,ry:10px,font-weight:600;

    files["<b>.tsx / .d.ts</b><br/>your source"] --> parse["<b>Parse</b><br/>OXC, in parallel"]
    parse --> merge["<b>Merge</b><br/>symbols across files"]
    merge --> resolve["<b>Resolve</b><br/>props per component"]
    resolve --> out["<b>Output</b><br/>canonical · rdt · toon"]

    class files source
    class parse,merge,resolve engine
    class out output
```

Caliper is pre-v1. [`docs/STATUS.md`](docs/STATUS.md) lists what works, and [`docs/edge-cases.md`](docs/edge-cases.md) the known gaps. Conditional types and mapped types over an unbound generic need a real type checker, which OXC deliberately isn't.

## Install

```bash
npm install --save-dev caliper-docgen                # CLI, with the prebuilt binary for your platform
npm install --save-dev caliper-docgen-vite-plugin    # Vite plugin
cargo install caliper-docgen                         # CLI from crates.io
```

Coming from `react-docgen-typescript`? [`MIGRATING.md`](MIGRATING.md) covers what's compatible today.

## Quick start

```bash
# Extract every component under src/ to a file
caliper-docgen extract --src src --out docgen.json

# Props of one component, as a table
caliper-docgen inspect Button --src src

# Re-extract on every change
caliper-docgen watch --src src

# Fail CI on extraction errors (--strict also fails on warnings)
caliper-docgen check --src src
```

Settings can also live in a `docgen.config.ts` (`srcDirs`, `htmlAttributes`, `cacheDir`, and more). `caliper-docgen --help` lists the rest, including `schema` for the JSON Schema of the output.

Types your props import from installed packages stay named references by default. `--load-dependencies` (config, NAPI and Vite option `loadDependencies`) loads the declarations those names point to.

## Vite plugin

```ts
// vite.config.ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { caliperDocgen } from 'caliper-docgen-vite-plugin'

export default defineConfig({
  plugins: [react(), caliperDocgen({ srcDirs: ['src'] })],
})
```

```ts
import docgen from 'virtual:caliper-docgen'

docgen.components['Button'].props
```

The module updates through HMR as components change. The plugin and the `caliper-docgen-napi` binding expose the canonical format only, not the `react-docgen-typescript` shape.

## Supported patterns

| Pattern                       | Example                                                           |
| ----------------------------- | ----------------------------------------------------------------- |
| Function and arrow components | `function Button(props: ButtonProps)`                             |
| `React.forwardRef`            | `forwardRef<HTMLButtonElement, ButtonProps>(...)`                 |
| Wrapped components            | `styled(Base)<ButtonProps>`                                       |
| Interface inheritance         | `interface ButtonProps extends HTMLAttributes<HTMLButtonElement>` |
| Intersections, `Omit`, `Pick` | `type ButtonProps = Omit<InputProps, 'value'>`                    |
| `ComponentPropsWithoutRef`    | `ComponentPropsWithoutRef<'button'>`                              |
| `VariantProps`                | `VariantProps<typeof buttonVariants>` (CVA, TV, PandaCSS)         |
| Discriminated unions          | `variant: 'filled' \| 'outlined' \| 'standard'`                   |
| JSDoc tags                    | `@description`, `@default`, `@deprecated`, `@internal`            |

Components declared through a generic wrapper call, such as Mantine's `factory<ButtonFactory>()`, are recognized from their type arguments. Not recognized yet: wrapper calls with no type argument, such as `lazy()`. Props that come from packages that aren't installed stay unresolved and are reported as warnings. [`docs/edge-cases.md`](docs/edge-cases.md) has the measured gaps.

## Documentation

- [Project page](https://orin-dx.com/tools/caliper/) on orin-dx.com.
- [Architecture](ARCHITECTURE.md): pipeline, crate map, invariants.
- [Decisions](docs/adr/README.md): why OXC, the cache format, panic containment, the planned TypeScript 7 backend.
- [Status](docs/STATUS.md), [edge cases](docs/edge-cases.md) and [benchmarks](docs/benchmarks.md).
- [Migrating](MIGRATING.md) from `react-docgen-typescript`.
- [Contributing](CONTRIBUTING.md) and [security](SECURITY.md).

## License

[FSL-1.1-MIT](LICENSE): free for any non-competing use, and each version becomes MIT two years after its release.
