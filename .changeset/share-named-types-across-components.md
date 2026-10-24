---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Faster on large style systems:** a named type, and an HTML element's attribute declaration, are resolved once per run and shared by every component that reaches them, as interface chains already were. A thread that needs a type another is resolving waits for it instead of resolving it too. Chakra with `--load-dependencies`: resolution CPU 3.56 s → 2.80 s (main: 2.75 s), peak RSS 395 → 350 MB (main: 372 MB).
