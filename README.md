<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/hero-dark.png">
    <img src="assets/brand/hero-light.png" alt="caliper, an Orin DX tool" width="360">
  </picture>
</p>

<p align="center">
  <b>React component prop extraction. TypeScript's checker decides the types, OXC reads the syntax.</b>
</p>

<p align="center">
  <a href="https://github.com/orin-dx/caliper/actions/workflows/ci.yml"><img src="https://github.com/orin-dx/caliper/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-FSL--1.1--MIT-blue.svg" alt="License" /></a>
</p>

---

`caliper-docgen` reads your components and reports each one's props: type, required or optional, default, and JSDoc. It is an alternative to `react-docgen-typescript` for Storybook, docs sites, and LLM tooling.

By default the types come from TypeScript's own checker: `tsrs`, a Rust port of the TypeScript 7.1 checker, is built in, so there is nothing to install. OXC supplies what the syntax says: which components a file declares, defaults, JSDoc and the order props and union members are written in ([ADR 0026](docs/adr/0026-the-checker-is-the-type-authority.md)). Where the checker cannot answer, a structural resolver that reads types from the syntax alone takes over, and `--no-checker` (`checker: false`) runs it on its own.

The checker costs time. Extracting the 938 components of an app that wraps Chakra UI and MUI takes about 6 s and 970 MiB with the checker, and 0.1 s and 200 MiB with `--no-checker` ([`docs/gate-measurements.md`](docs/gate-measurements.md#the-checker-is-the-default-s8)). The Storybook addon does not make a story wait for it: a story gets the structural props at once and the checker's within 150 ms, and the opening story's are fetched when `storybook dev` starts. The structural resolver alone takes 34 to 81 ms on whole libraries (Mantine, Chakra, Ant Design, shadcn/ui and React Spectrum at pinned commits, source only), 17 to 39 times faster than `react-docgen-typescript` on the same files; that is `--no-checker`, not the default, measured by hand ([`docs/performance.md`](docs/performance.md)); the weekly CI run records the default's time and memory. Whether the structural types are right is judged against TypeScript's checker, not against `react-docgen-typescript`: on the props both tools list, the checker contradicts the structural resolver on 0 to 1.09% and `react-docgen-typescript` on 0.006 to 5.02%, depending on the corpus (weekly run of 2026-11-06, [`docs/rdt-differences.md`](docs/rdt-differences.md)). Recall against `react-docgen-typescript` (the share of its reference components and props that Caliper also reports) is in [`docs/measurement.md`](docs/measurement.md); it is not a measure of correctness, because what `react-docgen-typescript` lists beyond Caliper is not necessarily right.

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

By default all of an element's inherited HTML attributes are exposed (what `react-docgen-typescript` does); `--html-attributes curated` keeps a compact subset, `none` none. `--format canonical` is the tool's own shape, `rdt` matches `react-docgen-typescript`, and `toon` is a compact format for LLM agents.

```mermaid
%%{init: {'theme': 'base', 'flowchart': {'curve': 'basis', 'nodeSpacing': 40, 'rankSpacing': 56}, 'themeVariables': {'fontFamily': 'Inter, ui-sans-serif, system-ui, sans-serif', 'fontSize': '14px', 'lineColor': '#94a3b8', 'edgeLabelBackground': '#ffffff'}}}%%
flowchart LR
    classDef source fill:#eef2ff,stroke:#6366f1,stroke-width:1.5px,color:#1e1b4b,rx:10px,ry:10px,font-weight:600;
    classDef engine fill:#f5f3ff,stroke:#8b5cf6,stroke-width:1.5px,color:#4c1d95,rx:10px,ry:10px;
    classDef output fill:#ecfdf5,stroke:#10b981,stroke-width:1.5px,color:#064e3b,rx:10px,ry:10px,font-weight:600;

    files["<b>.tsx / .d.ts</b><br/>your source"] --> parse["<b>Parse</b><br/>OXC, in parallel"]
    parse --> merge["<b>Merge</b><br/>symbols across files"]
    merge --> resolve["<b>Resolve</b><br/>TypeScript checker, structural fallback"]
    resolve --> out["<b>Output</b><br/>canonical · rdt · toon"]

    class files source
    class parse,merge,resolve engine
    class out output
```

Caliper is pre-v1. [`docs/STATUS.md`](docs/STATUS.md) lists what works, and [`docs/edge-cases.md`](docs/edge-cases.md) the known gaps. Conditional types and mapped types over an unbound generic need a type checker: the default asks one, and with `--no-checker` they are among those gaps.

## Install

```bash
npm install --save-dev caliper-docgen                # CLI, with the prebuilt binary for your platform
npm install --save-dev caliper-docgen-vite-plugin    # Vite plugin
npm install --save-dev caliper-docgen-storybook      # Storybook 11 addon
cargo install caliper-docgen                         # CLI from crates.io
```

Supported platforms: macOS (arm64, x64) and Linux glibc (arm64, x64). Windows is not supported; use WSL.

Coming from `react-docgen-typescript`? [`MIGRATING.md`](MIGRATING.md) covers what's compatible today.

## Quick start

```bash
# Extract every component under src/ to a file
caliper-docgen extract --src src --out docgen.json

# Props of one component, as a table
caliper-docgen inspect Button --src src

# Re-extract on every change (structural: watch does not run the checker)
caliper-docgen watch --src src

# Fail CI on extraction errors (--strict also fails on warnings)
caliper-docgen check --src src
```

`extract`, `check` and `inspect` ask the checker; `--no-checker` (config `checker: false`) skips it, and `--tsc-path` runs a checker binary instead (`typescript@7.1`'s `tsc`, or `tsrs`). Settings can also live in a `docgen.config.ts` (`srcDirs`, `htmlAttributes`, `cacheDir`, `checker`, and more). `caliper-docgen --help` lists the rest, including `schema` for the JSON Schema of the output.

Parsed declarations and the dependency index are cached under `node_modules/.cache/caliper-docgen`. `--no-cache` (config, NAPI, Vite and Storybook option `cache: false`) reads and writes nothing there; the output is the same.

Types your props import from installed packages are resolved by default: the declarations those names point to are loaded. `--no-load-dependencies` (config, NAPI and Vite option `loadDependencies: false`) leaves them as named references with an `UNRESOLVABLE_IMPORT` warning, and a project with no packages installed loads nothing either way. A package source contributing more than 30 props is then listed as one inherited layer with its notable props, as Storybook does; `--collapse-inherited-props all` does the same for the project's own sources and `none` lists every prop.

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

The module updates through HMR as components change. The plugin serves the structural resolver's output: it does not run the checker. The virtual module is the canonical format; the `injectDocgenInfo` option assigns `__docgenInfo` in the `react-docgen-typescript` shape (see the [plugin README](packages/vite-plugin/README.md)).

## Storybook

```ts
// .storybook/main.ts
export default {
  framework: '@storybook/react-vite',
  addons: [{ name: 'caliper-docgen-storybook', options: { srcDirs: ['src'] } }],
}
```

Storybook 11's docgen server then takes each component's argTypes from caliper-docgen, which asks its built-in TypeScript checker for the component a story shows (`checker: false` turns it off), and hands anything caliper can't document to React's own docgen. See [`packages/storybook`](packages/storybook/README.md).

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
- [Status](docs/STATUS.md), [edge cases](docs/edge-cases.md), [benchmarks](docs/benchmarks.md) and [where Caliper's types differ from RDT and the checker](docs/rdt-differences.md).
- [Migrating](MIGRATING.md) from `react-docgen-typescript`.
- [Contributing](CONTRIBUTING.md) and [security](SECURITY.md).

## License

[FSL-1.1-MIT](LICENSE): free for any non-competing use, and each version becomes MIT two years after its release.
