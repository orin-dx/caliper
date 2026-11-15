---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With the checker on, a prop type that would take more than 256 KiB of JSON is now cut and shown by its name (`truncated`, limit `depth`), as types nested too deep already are. Only such props change; every smaller prop type is unchanged. A style system kept in the project's own source (Chakra UI's `SystemStyleObject` props such as `_hover` and `css`) expanded to about 25 MB per prop, and an extract of Chakra's own source took 42 GB of memory.
