---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

`Pick` and `Omit` now work when their first argument is itself generic (`Pick<Gen<number>, "x">`) and when `Omit` removes `keyof Other`. Before, a generic first argument dropped the props with no warning, and `keyof` omitted nothing. Known HTML attribute interfaces named inside these utilities no longer raise an unresolvable-type warning.
