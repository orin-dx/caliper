---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Attributes added to React's interfaces by `declare module "react" { interface HTMLAttributes<T> { … } }` now reach every HTML-based component, as TypeScript merges them: `tw` from `next/server`'s bundled OG types shows up on shadcn's components when `--load-dependencies` is on and `--html-attributes` is `full`. Dependencies are searched for these declarations through the re-exports of every module the project imports, so a run with `--load-dependencies` reads a few more declaration files.
