---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A component whose name starts with an underscore is now found: `export default function _3D(props: { size?: "L" | "M" | "S" })`, which is how a file named `3D.tsx` has to spell its function, was skipped with no note before, and so were `_Card` and `_Box`. A name needs a capital letter after the underscore and any digits (`_helper` and `_3d` are still not components), and the function still has to render something. The parse cache is rebuilt once.
