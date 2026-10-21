---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Removed eight options that were accepted but never used: `reactVersion`, `crossPackage`, `pandacssOutdir`, `variantFunctions`, `vanillaExtract`, `resolveComplexTypes`, `extraPaths` and `knownTypeOverrides`. This also removes the CLI flags `--react-version` and `--no-cross-package`, and the `reactVersion` option of the Vite plugin. A `docgen.config.ts` that still names one now fails with an error naming the field, instead of looking honored. Extraction output is unchanged.
