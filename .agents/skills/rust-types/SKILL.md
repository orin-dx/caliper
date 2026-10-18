---
name: rust-types
description: Apply when defining new public structs or enums.
---

**Goal:** Every public type is immediately usable, visible in error messages, and extendable without semver breaks.

## Checklist before merging a new `pub struct` or `pub enum`

- `#[derive(Debug)]` — always
- `impl Display` — if it appears in messages, CLI output, or diagnostic text
- `impl std::error::Error` — if it represents a failure condition
- `impl AsRef<str>` — on string newtypes
- `#[non_exhaustive]` — on enums that will gain variants; adding one without this is a semver break
- `#[must_use]` — on functions whose return value the caller must not silently drop

## Wire or don't define

A new abstraction needs a real call site in the same PR — not a follow-up ticket. An unused type adds noise and false confidence. If the use case isn't clear yet, leave a note in the doc instead.

Reference: `crates/core/src/types/diagnostic.rs` — canonical complete type in this project.
