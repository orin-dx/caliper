---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **A wrapper call that names its props type keeps those props when it also receives a component:** `withContext<HTMLDivElement, OverlayProps>(BaseOverlay, "overlay")` has the props of `OverlayProps`, not only those of `BaseOverlay`. Chakra's `CodeBlockOverlay` goes from 1 prop to 843 and six other code-block components gain their style and HTML props.
