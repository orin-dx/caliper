---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Compound components built with `Object.assign(Root, { Item })` or by assigning `Root.Item = …` are now found under their public names, with inline sub-components included, instead of being reported as the inner function or dropped without a note. No action needed.
