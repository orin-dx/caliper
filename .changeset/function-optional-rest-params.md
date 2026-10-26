---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-vite-plugin: minor
npm/caliper-docgen-storybook: minor
---

Function props now show which arguments you can leave out and which repeat, so `(value: string, extra?: number, ...rest: boolean[]) => void` no longer reads as `(value: string)`. The JSON output moves to schema version 3: update `caliper-docgen`, the Vite plugin and the Storybook addon together, and a mismatched install stops with a message naming both versions.
