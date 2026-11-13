---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
---

`caliper-docgen extract` ends sooner: the process is about to exit, so it no longer waits to free the checker run's memory, release its snapshot and wind down its runtime (about 35-50 ms on a large project). The output is unchanged. This is a `Teardown::OneShot` run; every other caller (Storybook, watch, the binding, `check`, `inspect`) keeps the default `Teardown::Session` and frees and releases as before.
