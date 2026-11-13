---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A component its module exports under exactly one other name is now reported under that name: `export { _Card as Card }` is `Card`, where it was `_Card` (and `export { Wrapped as DialogTrigger }` is `DialogTrigger`, where it was `Wrapped`). A component with a `displayName`, one exported under its own name or under several names, one reached through a parent, and one whose export name another component of the file already has keep the name they had. The canonical JSON's `exports` is unchanged and still identifies the component (ADR 0027), and the module's own binding is kept in the new `ComponentMapping::local_name`, so `typeof _Card` inside the file and through the export from another module still resolves.

Breaking: the `displayName` (and output key) of such a component changes, so a reader that looks it up by the old internal name (`components["_Card"]`) no longer finds it. `ComponentMapping` gains a public field, `local_name`. The canonical schema version is unchanged (7).

Action: look a component up by its export (`exports`), or by the new name. The declaration cache is rebuilt once (schema 38).
