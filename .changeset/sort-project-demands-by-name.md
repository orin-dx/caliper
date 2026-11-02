---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Runs that load dependency declarations are about 10 ms faster on a project the size of shadcn/ui (177 to 164 ms wall time on a loaded machine, output unchanged): a file's package demands are sorted by specifier and name instead of by a path they all share.
