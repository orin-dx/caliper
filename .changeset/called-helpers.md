---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Functions used only as functions are no longer reported as components:** a PascalCase function that returns JSX but is only ever called in its file (`ArrayType(type)`), or that takes a second parameter such as a ref and is never rendered or passed to a component wrapper (a render callback like `{render: TableDropIndicatorWrapper}`), is now a skipped candidate with the reason. Exported functions are unaffected. On React Spectrum this removes 25 false components.
