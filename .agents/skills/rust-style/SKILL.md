---
name: rust-style
description: Apply when writing or modifying Rust code in this project.
---

**Goal:** Code correct by construction, readable under churn, performant in hot paths.

## Project-specific decisions

These are non-negotiables, not preferences:

- Internal maps: `FxHashMap`. JSON-output maps: `BTreeMap` (deterministic key ordering).
- Type/prop names: `CompactString` — avoids heap allocation for strings ≤24 bytes.
- No `unwrap()` outside `#[cfg(test)]` — use `?` and let callers decide what to do with failure.

## Control flow

`let...else` keeps the happy path flat:

```rust
let Some(entry) = map.get(key) else { return None };
let Ok(parsed) = input.parse::<u32>() else { continue };
```

## Matching

No wildcards — a compile error on unhandled variants is a feature:

```rust
match status { Status::Ok => .., Status::Err => .. }  // compiler tells you when Status grows
match status { Status::Ok => .., _ => .. }             // silently misses new variants
```

Full `match` over `matches!` for the same reason.

## Types

Newtypes when a bare type allows confusion. Enums when a bool parameter hides intent:

```rust
struct ScopedKey(CompactString);         // not: String
enum Visibility { Public, Private }      // not: is_public: bool
```

Explicit destructuring gets compiler errors when structs change:

```rust
let ComponentEntry { display_name, file_path, .. } = entry;
```

## Naming

Shadow variables through transformations — no `raw_`/`parsed_`/`trimmed_` prefixes:

```rust
let path = args.path;
let path = path.canonicalize()?;
let path = path.join("config.toml");
```

## Comments

Only when the WHY is non-obvious: a hidden constraint, a workaround, a non-obvious invariant. Not what the code does.
