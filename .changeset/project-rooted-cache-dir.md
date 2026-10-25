---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **The default cache directory belongs to the project, not the working directory:** with no `cacheDir`, the caches live in `node_modules/.cache/caliper-docgen` under the nearest ancestor of the first source directory that has a `node_modules` or a `package.json`. Running from another directory no longer misses the project's cache or creates a `node_modules` there, and the CLI, the Vite plugin and the Storybook addon share one cache. A source directory in no project uses a directory in the OS temp directory.
- **A run that learned nothing new no longer rewrites the dependency index:** the project's own imports are resolved every run and never reach the index, yet they made every run write it again (shadcn with dependencies, warm: 97.7 → 93.0 ms; Chakra: 285.7 → 259.9 ms).
