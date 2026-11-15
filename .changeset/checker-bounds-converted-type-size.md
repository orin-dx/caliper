---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With the checker on, a type made of more than 18,724 types (counting itself and every type inside it) is now cut and shown by its name, as types nested too deep already are. It is reported as a depth truncation (`truncated`, limit `depth`), because a limit kind of its own would change the output schema. No type under 256 KiB of JSON is ever cut, since each type takes at least 14 bytes; on real projects a type averages about 40 bytes, so the cut falls near 0.75 MB of JSON (Chakra's largest prop type left whole is 732 KB). Only types past that count change; every smaller one is unchanged. A style system kept in the project's own source (Chakra UI's `SystemStyleObject` props such as `_hover` and `css`) expanded to about 25 MB per prop, and an extract of Chakra's own source took 42 GB of memory.
