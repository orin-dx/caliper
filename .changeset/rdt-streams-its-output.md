---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

`extract --format rdt` on a large project is more than twice as fast and needs about a quarter of the memory, with byte-identical output: 1.8 s and 1.1 GiB instead of 4.3 s and 3.7 GiB on the largest project measured, and 1.4 s and 0.9 GiB instead of 3.9 s and 3.6 GiB on the 787 Chakra components. The document is written as it is rendered instead of being built whole, and each distinct prop (a few thousand, shared by half a million occurrences) is rendered once.
