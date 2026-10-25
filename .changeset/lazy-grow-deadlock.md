---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

- **A lazy session no longer deadlocks when requests load dependencies at the same time:** `Revision::grow` held its lock while the dependency load fanned out with rayon. A thread waiting for that fan-out stole another queued request, which called `grow` and blocked on the lock its own thread held, while the other workers blocked on it too (found as a `lazy_parity` run over the large corpora that sat at 0% CPU for 100 minutes). The load now runs on a thread and a pool of its own.
