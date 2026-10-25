---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

- **A large project's output assembles about 10 ms faster:** the components each diagnostic affects are sorted in parallel instead of one diagnostic at a time (Chakra with dependencies lists about 340,000 of them). The output is unchanged.
