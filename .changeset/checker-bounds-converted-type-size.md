---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

**An extract with the checker on no longer expands a huge type into megabytes of output.** A type made of more than 18,724 nested types is cut to its name, as types nested too deep already are.

- Why it matters: an extract of Chakra's own source took 42 GB on 0.11.0, because each style-system prop (`_hover`, `css`) expanded to about 25 MB.
- Limit: the cut shows as a depth truncation (`truncated`, limit `depth`).
- Limit: only types past that size change. Chakra's largest prop type left whole is 732 KB of JSON.
