---
cargo/caliper-docgen: minor
npm/caliper-docgen: minor
---

- **The `lsp` subcommand is removed:** it was a scaffold that advertised `hoverProvider: false` and answered nothing but `initialize`, `shutdown` and `exit`. Editors get hover and completion from the TypeScript language server; the documentation view of a component is available to tools through the lazy session API.
