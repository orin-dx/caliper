---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

The weekly measurement now reports each run's own peak memory instead of one shared maximum, and measures recall against a cleaned reference: react-docgen-typescript's hooks, route handlers, helpers and context objects are left out, and a component it lists under several re-exporting files counts once. The record counts what was left out and what was paired, so a change in the reference is visible. Library output is unchanged.
