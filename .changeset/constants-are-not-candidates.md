---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A constant such as `MENU_OPTIONS` or `ORIGIN` is no longer reported as a skipped component candidate: on shadcn/ui and Ant Design that removes 211 of the 398 skip notes (163 of 252 and 48 of 146), with the extracted components unchanged.
