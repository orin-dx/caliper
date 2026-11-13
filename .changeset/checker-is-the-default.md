---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-vite-plugin: patch
---

The built-in TypeScript checker now answers by default (ADR 0026). `extract`, `check` and `inspect` merge its answer over the structural one, prop by prop, and wait for it. A native session and the Storybook addon ask it for each component a request names, and still show the structural props first when it is slower than `semanticWaitMs` (150 ms). `--no-checker`, or `checker: false` in `docgen.config.ts`, the native options or the addon's options, runs the structural resolver alone, as before. `watch`, `extractAll` and the Vite plugin stay structural.

The checker costs time and memory. `extract` of an app with 938 Chakra UI and MUI components takes about 6 s and 970 MiB, against 0.1 s and 200 MiB with `--no-checker`. A session that answered every one of them peaks at about 1 GB, against 380 MiB without the checker: each answer the session keeps now shares its props with the others, where before each kept its own copy, about 6.7 MiB per Chakra component.

Breaking: `--experimental-semantic-types`, the config's `experimentalSemanticTypes` and the `semanticTypes` option of the native binding and the Storybook addon are removed. The CLI and the config reject the old names as unknown, and the addon rejects `semanticTypes` by name. `PipelineOptions::experimental_semantic_types` is now `PipelineOptions::checker`, and it is `true` by default.

Action: drop `--experimental-semantic-types`, `experimentalSemanticTypes: true` and `semanticTypes: 'on-demand'`, since the checker now runs without them. Replace `semanticTypes: 'off'`, or a run that relied on the old structural default, with `checker: false` or `--no-checker`.
