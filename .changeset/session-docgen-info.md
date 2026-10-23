---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen-napi: minor
---

- **`sessionDocgenInfo(sessionId, keys)`:** returns the named components of a watch session's current revision in react-docgen-typescript's `__docgenInfo` shape (`RdtComponentDoc`), the same shape `--format rdt` writes. Keys the session doesn't have are left out.
- **Rust API:** `caliper_docgen_core::pipeline::rdt` holds the RDT projection (`serialize`, `docgen_info`, `type_json`, `omits`), and `WatchSession::current_output` shares the current output without copying it.
