---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

An interface that extends a type imported through a barrel file (`import type { UnstyledProp } from "../styled-system"` where `index.ts` re-exports it) now picks up that type's props. Before, the barrel was treated as the declaring file, so the props were dropped with an `UNRESOLVABLE_IMPORT` warning. On the Mantine and Chakra sources this raises the share of react-docgen-typescript's own props that Caliper reports from 13% and 18% to 86% and 85%.
