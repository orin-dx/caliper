---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

An unreadable `@types/react` or TypeScript lib file now raises an `IO_ERROR` diagnostic instead of being parsed as an empty file and cached that way. `--format rdt` and `--format storybook` now exit with an error if serialization fails instead of printing nothing.
