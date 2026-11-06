---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

On Windows, discovered and watched paths no longer carry the `\\?\` prefix, so they match the `C:\…` form that the Storybook addon and Vite plugin compare against. No action needed.
