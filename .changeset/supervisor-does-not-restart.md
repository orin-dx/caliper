---
cargo/caliper-docgen-tsc-client: minor
---

The `tsc` supervisor no longer restarts a crashed process, because nothing used it to; `caliper-docgen-semantic` runs its own restart policy. `Supervisor::request` is gone (send requests through `Supervisor::client()`, which was already the way to do it), as are the error variant `TscClientError::RestartStormDetected` and the supervisor's restart cap and backoff. A session that ends stays ended, and `Supervisor::is_alive` is false from then on; spawn a new `Supervisor` to start again.
