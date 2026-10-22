---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **`tsconfigPath` is read the way TypeScript reads it:** block and line comments, `extends`, `baseUrl` and `paths` wildcards. Before, a hand-written reader understood only line comments and ignored `extends`. Any config it could not parse silently produced no aliases, so every aliased import failed to resolve with nothing pointing at the config.
- **A tsconfig that cannot be read or parsed is reported once,** as a warning naming the file.
- **React Aria's `RenderProps<T>`** gives `className`, `style` and `children` as a value or a function of the render state. Before, `className` and `style` lost their function form and `children` was dropped.
- **A parse-cache file over 256 MB is reported and ignored** rather than read into memory.
