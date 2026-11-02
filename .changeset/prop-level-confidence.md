---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
---

A prop now says why its type is partly a guess: the canonical JSON gains `degradedBy` on a prop whose type holds an opaque member, and the TOON output (`--format toon`) lists each prop's declaring type, quotes string literals in single quotes, shows the names a collapsed inherited layer declares, and adds a `degradedBy` column when a prop has one. A host that works in process can ask the session for one component as TOON with `sessionComponentToon(sessionId, key)` instead of rendering a whole project.
