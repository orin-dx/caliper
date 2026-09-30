<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/hero-dark.png">
    <img src="assets/brand/hero-light.png" alt="caliper, an Orin DX tool" width="360">
  </picture>
</p>

<p align="center"><code>oxc-react-docgen</code></p>

React component prop extraction powered by [OXC](https://oxc.rs). Parses TypeScript natively in Rust — no TypeScript compiler program, no type-checking pass, no startup tax.

> **Status:** Core extraction, NAPI bindings, Vite plugin, CLI, and `docgen.config.ts` loading are complete.

## Why

`react-docgen-typescript` spins up a full TypeScript `Program` to extract props. On a mid-size design system that takes seconds — enough to noticeably delay Storybook startup and make HMR feel sluggish.

OXC parses each file in parallel with no type-checking pass:

- **Cold extraction, 21 real component libraries:** 50ms ([full breakdown](docs/benchmarks.md))
- **Incremental single-file update:** ~2ms — react-docgen-typescript has no equivalent API; every edit re-pays the full cold cost

The CLI can emit an RDT-compatible shape via `--format rdt` (see [MIGRATING.md](MIGRATING.md)). The Vite plugin and NAPI binding currently expose this tool's own canonical format only — not a drop-in RDT replacement at that layer yet.

## Install

> Not yet on npm. No prebuilt per-platform binaries exist yet — build from source:

```bash
git clone https://github.com/orin-dx/oxc-react-docgen
cd oxc-react-docgen
pnpm install
pnpm --filter @oxc-react-docgen/napi run build:napi   # builds the native addon for your platform
pnpm --filter @oxc-react-docgen/vite-plugin build
```

Use `pnpm run build:napi`, not a bare `cargo build` — it invokes `napi build`, which places the compiled addon exactly where `packages/napi/index.js`'s loader expects it.

Migrating off `react-docgen-typescript`? See [MIGRATING.md](MIGRATING.md) for what's compatible today.

## Vite plugin

```ts
// vite.config.ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { oxcReactDocgen } from '@oxc-react-docgen/vite-plugin'

export default defineConfig({
  plugins: [react(), oxcReactDocgen({ srcDirs: ['src'] })],
})
```

Metadata is available on the `virtual:oxc-react-docgen` module and pushed via HMR as components change:

```ts
import docgen from 'virtual:oxc-react-docgen'

docgen.components['Button'].props
// {
//   variant: {
//     type: { kind: 'literalUnion', members: ['default', 'destructive', 'outline'], hasDefault: true },
//     required: false,
//     defaultValue: { value: '"default"', computed: false },
//     description: '',
//   },
//   ...
// }
```

## CLI

A single Rust binary (`crates/cli`) — argument parsing, output formatting, and `--format rdt`/`--format storybook` serialization all live there. `@oxc-react-docgen/cli` (once published) is a thin npm wrapper that execs the platform-appropriate compiled binary — no reimplemented logic, so there's exactly one place CLI behavior can drift.

```bash
# Once published: npx @oxc-react-docgen/cli extract --src src/ --out docgen.json
# From source today: ./target/release/oxc-react-docgen (after cargo build --release)

# One-shot extraction
oxc-react-docgen extract --src src/ --out docgen.json

# Inspect a single file
oxc-react-docgen inspect src/components/Button.tsx

# Watch mode
oxc-react-docgen watch --src src/

# Shell completions
oxc-react-docgen completions zsh > ~/.zsh/completions/_oxc-react-docgen

# Full HTML attribute expansion instead of the default curated subset
oxc-react-docgen extract --src src/ --html-attributes full
```

Settings can also live in `docgen.config.ts` (`srcDirs`, `htmlAttributes`, `reactVersion`, `crossPackage`, and more — see `crates/cli/src/config.rs` for the full schema) instead of CLI flags.

## Output shape

```json
{
  "components": {
    "Button": {
      "displayName": "Button",
      "filePath": "src/components/Button.tsx",
      "props": {
        "variant": {
          "type": { "kind": "literalUnion", "members": ["default", "destructive", "outline"], "hasDefault": true },
          "required": false,
          "defaultValue": { "value": "\"default\"", "computed": false },
          "description": "Visual style variant.",
          "tags": {}
        }
      },
      "inheritance": [
        { "typeName": "ButtonHTMLAttributes", "htmlElement": "button", "omitted": [], "totalProps": 147 }
      ],
      "notableInherited": {
        "onClick": { "type": { "kind": "eventHandler", "eventType": "MouseEvent<HTMLButtonElement>" }, ... },
        "disabled": { "type": { "kind": "boolean" }, ... }
      },
      "discriminantProp": null
    }
  },
  "diagnostics": [],
  "stats": { "filesParsed": 3, "durationMs": 12, "dtsCacheHits": 2, ... }
}
```

## Supported patterns

| Pattern                    | Example                                                           |
| -------------------------- | ----------------------------------------------------------------- |
| Function components        | `function Button(props: ButtonProps)`                             |
| Arrow components           | `const Button = (props: ButtonProps) => ...`                      |
| `React.forwardRef`         | `forwardRef<HTMLButtonElement, ButtonProps>(...)`                 |
| HOC-wrapped                | `styled(Base)<ButtonProps>`                                       |
| Interface inheritance      | `interface ButtonProps extends HTMLAttributes<HTMLButtonElement>` |
| Intersection aliases       | `type ButtonProps = BaseProps & { variant?: string }`             |
| `Omit` / `Pick`            | `type ButtonProps = Omit<InputProps, 'value'>`                    |
| `ComponentPropsWithoutRef` | `ComponentPropsWithoutRef<'button'>`                              |
| `VariantProps`             | `VariantProps<typeof buttonVariants>` (CVA / TV / PandaCSS)       |
| Discriminated unions       | MUI-style `variant: 'filled' \| 'outlined' \| 'standard'`         |
| JSDoc tags                 | `@description`, `@default`, `@deprecated`, `@internal`            |

## Repository

```
crates/core/          pure extraction logic (no I/O, no async, Send + Sync)
crates/binding/       thin NAPI wrapper over core
crates/cli/           clap + miette CLI
packages/napi/        @oxc-react-docgen/napi (dev binary loader + TS types)
packages/vite-plugin/ @oxc-react-docgen/vite-plugin
packages/cli/         @oxc-react-docgen/cli (npx wrapper — execs crates/cli's binary, no reimplemented logic)
apps/validate/        accuracy comparison harness (react-docgen, rdt, ours)
fixtures/             real library .d.ts/.tsx files (shadcn, MUI, Chakra, Mantine, etc.)
```

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) — setup, code style, commit conventions.  
[ARCHITECTURE.md](ARCHITECTURE.md) — pipeline, modules, key design decisions.

## License

AGPL-3.0-or-later — see [LICENSE](LICENSE).
