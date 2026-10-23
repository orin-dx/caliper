---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Module resolution follows TypeScript more closely:** without a configured tsconfig, each file uses the nearest `tsconfig.json` (its `paths`, `baseUrl` and `extends`). The tsconfig's `customConditions` apply, `.d.mts`/`.d.cts` declarations resolve, and an import written as `./button.js` finds `button.ts`. On React Spectrum, unresolved types drop from 586 to 180; on shadcn, unresolved imports from 205 to 16.
- **Package declarations:** a package whose `exports` list `"import"` before `"types"` now resolves to its declarations, and a declaration path that doesn't exist (an unbuilt workspace package) is reported as unresolved instead of silently resolving to a missing file.
- **Breaking (Rust API):** `resolve_package_dts_path` takes the `PipelineOptions`.
