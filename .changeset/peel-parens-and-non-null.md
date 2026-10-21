---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

Components wrapped in parentheses or a trailing `!`, such as `export const Button = (forwardRef<El, Props>(...))`, are now detected. They were silently skipped before.
