---
npm/caliper-docgen-napi: minor
cargo/caliper-docgen-napi: minor
---

The native addon no longer exports `sessionStats`, `sessionComponentKeys` and the `JsLazyStats` type: no Caliper package called them. A host that read a lazy session's counts or its component keys through them should read the keys from `initializeSessionIndex`, which lists every component of the session. The other session functions and the addon options are unchanged.
