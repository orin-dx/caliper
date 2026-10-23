---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Components that inherit one interface share its resolved props instead of copying them,** and the interface is resolved once per run for every component whose context would resolve it the same way. Output is unchanged; on large design systems extraction is up to 40% faster and peak memory up to 45% lower.
- **`ComponentEntry::props` and `notable_inherited` hold `Arc<ParsedProp>`** (Rust API only). Read a prop as before; change one through `Arc::make_mut`, which copies it first when another component shares it, so the change stays with that component.
