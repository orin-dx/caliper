---
cargo/caliper-docgen-core: patch
---

Large design systems extract faster: shadcn/ui with its dependencies went from 153 ms to 127 ms with every HTML attribute listed, and from 124 ms to 92 ms without, because a `ComponentProps<typeof X>` reference no longer compares every component's path. The output is unchanged. No action needed.
