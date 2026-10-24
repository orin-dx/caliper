---
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-vite-plugin: minor
---

- **The Storybook addon documents more by default:** it reads dependency types (`loadDependencies`, default `true`) and lists every inherited prop (`collapseInheritedProps`, default `'none'`), so library-typed props resolve and fewer components fall back to Storybook's own docgen. New options: `htmlAttributes` (`'curated'` | `'full'` | `'none'`), `loadDependencies`, `collapseInheritedProps`.
- **Fallbacks are visible:** a component handed to Storybook's docgen is logged once as a warning with the diagnostics that caused it.
- **Edits to a tsconfig, `package.json` or lockfile reach the addon** like a source edit, instead of being missed until restart.
- **The Vite plugin logs extraction diagnostics** once each (`logLevel`, `onDiagnostic`) and takes `htmlAttributes`.
- **Windows paths work in the Vite plugin:** `C:\` source directories and backslash file paths are recognised.
