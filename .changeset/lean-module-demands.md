---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Runs that load dependency declarations hold about 6% less memory on a project the size of shadcn/ui (330 to 313 MiB peak, output unchanged): the search for React augmentations behind a package a project imports resolves it once per directory scope, and reads the re-exports of the files it passes through without keeping their parse.
