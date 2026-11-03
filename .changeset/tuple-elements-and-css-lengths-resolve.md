---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Tuple props now show the real type of each element, including labelled and optional ones such as `[tone: Tone, count?: number]`, and a CSS length like `React.CSSProperties['width']` now reads `string | number` even when dependencies are not loaded. No action needed.
