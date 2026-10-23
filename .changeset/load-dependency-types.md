---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

- **Opt-in loading of package types:** `--load-dependencies` (config `loadDependencies`, NAPI and Vite plugin `loadDependencies`, Rust `PipelineOptions::load_dependencies`) resolves the types your props import from installed packages, loading only the declarations those names reach instead of whole packages. With it on, shadcn's components gain 11,394 props and 349 fewer have none, and "Cannot resolve type" warnings drop from 378 to 55; chakra's drop from 323 to 25 and antd's from 112 to 58. A run stays 29–42× faster than react-docgen-typescript on the same projects. A type that still can't be found says why: its module didn't resolve, the package ships no types, or its types don't export it. Off by default; with it off, output is unchanged.
- **Breaking (Rust API):** `LexedExport::LocalDeclaration` gains `declared_as`, the local name behind `export { Base as Name }`. The parse cache schema moves to 16.
