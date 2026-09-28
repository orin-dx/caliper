---
name: rustdoc
description: Apply when writing doc comments on public Rust items.
---

**Goal:** Public API is usable from docs.rs without reading source.

## Rules (RFC 1574)

First line: single sentence, third-person present, ends with period.

```rust
/// Returns the resolved component entry.   // correct
/// Return the component entry.             // wrong — imperative
/// This returns the component entry.       // wrong — wordy
```

Use `///`, not `/** */`. Use `//!` only for crate/module-level docs at the top of the file.

Link types by name, not just backticks:

```rust
/// Returns [`ComponentEntry`], or `None` if resolution failed.
```

Add sections only when they carry information: `# Examples`, `# Panics`, `# Errors`, `# Safety`. Every public `fn` gets at least one example. Hide boilerplate with `# `:

````rust
/// # Examples
/// ```
/// # use oxc_react_docgen_core::pipeline::PipelineOptions;
/// let opts = PipelineOptions::default();
/// ```
````

American English spelling.
