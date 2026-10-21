---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Two files that export an enum with the same name now give the same output on every run: the first file path in sorted order wins, and a `DUPLICATE_ENUM_NAME` warning names both files. Before, hash order picked the winner and nothing was reported. Enum keys on Windows paths are also split correctly. The on-disk parse cache is rebuilt once (schema 5).
