---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Faster extraction on large projects:** resolved components move into the output instead of being copied, the canonical JSON is written in parallel, and the CLI uses mimalloc. Mantine extracts in 41 ms instead of 83 ms and Spectrum in 47 ms instead of 63 ms, with identical output.
- **Fix:** `--out` to a device or FIFO such as `/dev/null` failed with a permission error, because the atomic write staged its temp file in `/dev`. Such targets are now written in place.
