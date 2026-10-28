---
cargo/caliper-docgen-core: patch
---

Polymorphic Mantine components now show their `component` prop as `ElementType` instead of leaking the internal type parameter `C | ElementType`.
