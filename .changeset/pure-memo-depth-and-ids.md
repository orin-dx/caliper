---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

- **Resolution with `--load-dependencies` is up to 19% faster on Chakra and uses 11% less memory:** a type reached at several depths is resolved once instead of once per depth, and the resolver's cycle sets and read logs hold numeric ids instead of long key strings with shared reference counts. The output is unchanged.
