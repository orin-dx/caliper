# Agent: Repository Setup (Phase 0)

# Model: claude-sonnet-4-6

# Runs: First, blocks all other agents

## Mission

Bootstrap the complete repository skeleton. No logic, no algorithms — just structure, tooling, and the workspace Cargo.toml so every subsequent agent has a stable foundation.

## Acceptance Criteria

- `cargo build` succeeds on empty crates
- `cargo test` runs with zero failures
- `cargo clippy -- -D warnings` passes
- CI workflow runs on push
- All agents can start their work without file conflicts

## Files to Create

### /Cargo.toml (workspace root)

```toml
[workspace]
members = ["crates/core", "crates/napi", "crates/cli", "packages/rolldown-plugin"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
authors = ["oxc-react-docgen contributors"]
license = "MIT"

[workspace.dependencies]
# OXC — pin all to same patch version
oxc_allocator = "0.60"
oxc_parser = "0.60"
oxc_ast = "0.60"
oxc_span = "0.60"
oxc_module_lexer = "0.60"
oxc_resolver = "2.0"

# Core data
compact_str = { version = "0.8", features = ["serde"] }
rustc-hash = "2.1"
dashmap = "6.1"
arc-swap = "1.7"
camino = { version = "1.1", features = ["serde1"] }
indexmap = { version = "2.2", features = ["serde"] }
dirs = "5.0"

# Serde
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
rmp-serde = "1.3"

# Error handling
thiserror = "2.0"
miette = "7.2"
anyhow = "1.0"

# Parallelism
rayon = "1.10"
tokio = { version = "1.40", features = ["rt-multi-thread", "process", "io-util", "sync", "macros"] }

# File system
ignore = "0.4"

# LSP (Tier 4 scaffold — not implemented)
lsp-types = "0.95"

# CLI
clap = { version = "4.5", features = ["derive", "env", "wrap_help"] }
clap_complete = "4.5"
clap_mangen = "0.2"
indicatif = "0.17"
tracing-indicatif = "0.3"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
owo-colors = "4.1"
watchexec = "5.0"

# NAPI
napi = { version = "2.16", features = ["napi4"] }
napi-derive = "2.16"

# Dev
divan = "0.1"
insta = { version = "1.42", features = ["json", "redactions"] }
pretty_assertions = "1.4"

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1

[profile.bench]
inherits = "release"
debug = true
```

### /crates/core/Cargo.toml

```toml
[package]
name = "oxc-react-docgen-core"
description = "Fast React prop extraction via OXC — core library"
version.workspace = true
edition.workspace = true

[dependencies]
oxc_allocator.workspace = true
oxc_parser.workspace = true
oxc_ast.workspace = true
oxc_span.workspace = true
oxc_module_lexer.workspace = true
oxc_resolver.workspace = true
compact_str.workspace = true
rustc-hash.workspace = true
dashmap.workspace = true
arc-swap.workspace = true
camino.workspace = true
indexmap.workspace = true
dirs.workspace = true
serde.workspace = true
serde_json.workspace = true
rmp-serde.workspace = true
thiserror.workspace = true
miette.workspace = true # NO fancy feature — just structured errors
rayon.workspace = true
tokio.workspace = true
ignore.workspace = true

[dev-dependencies]
divan.workspace = true
insta.workspace = true
pretty_assertions.workspace = true

[[bench]]
name = "extraction"
harness = false
```

### /crates/napi/Cargo.toml

```toml
[package]
name = "oxc-react-docgen-napi"
version.workspace = true
edition.workspace = true

[lib]
crate-type = ["cdylib"]

[dependencies]
oxc-react-docgen-core = { path = "../core" }
napi.workspace = true
napi-derive.workspace = true
serde_json.workspace = true

[build-dependencies]
napi-build = "1.0"
```

### /crates/cli/Cargo.toml

```toml
[package]
name = "oxc-react-docgen"
description = "Fast React prop extraction — CLI"
version.workspace = true
edition.workspace = true

[[bin]]
name = "oxc-react-docgen"
path = "src/main.rs"

[dependencies]
oxc-react-docgen-core = { path = "../core" }
clap.workspace = true
clap_complete.workspace = true
clap_mangen.workspace = true
miette = { workspace = true, features = ["fancy"] }
anyhow.workspace = true
indicatif.workspace = true
tracing-indicatif.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
owo-colors.workspace = true
watchexec.workspace = true
serde_json.workspace = true
camino.workspace = true

[dev-dependencies]
insta.workspace = true
```

### /crates/core/src/lib.rs (stub)

```rust
//! oxc-react-docgen-core
//!
//! Fast React prop extraction powered by OXC.
//! Zero terminal dependencies — usable from NAPI, CLI, and Rolldown natively.

pub mod types;
pub mod extractor;
pub mod import_map;
pub mod known;
pub mod resolver;
pub mod pipeline;
pub mod cache;
pub mod react_types;

pub use types::*;
pub use pipeline::{ExtractionPipeline, PipelineOptions};
```

### /crates/core/src/types.rs (stub — Phase 1a agent fills this)

```rust
// STUB — Phase 1a (Types Agent) owns this file
// Do not edit; leave it empty for now
```

### /crates/core/src/extractor.rs (stub)

```rust
// STUB — Phase 2a (Extractor Agent) owns this file
```

### Similar stubs for: import_map.rs, known.rs, resolver.rs, pipeline.rs, cache.rs, react_types.rs

### /crates/napi/src/lib.rs (stub)

```rust
// STUB — Phase 4a (NAPI Agent) owns this file
#![allow(unused)]
```

### /crates/napi/build.rs

```rust
fn main() {
    napi_build::setup();
}
```

### /crates/cli/src/main.rs (stub)

```rust
fn main() {
    println!("oxc-react-docgen CLI — coming soon");
}
```

### /.github/workflows/ci.yml

```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --check
      - run: cargo clippy -- -D warnings
      - run: cargo test --workspace
      - run: cargo bench --workspace --no-run # compile check only in CI

  bench:
    runs-on: ubuntu-latest
    if: github.ref == 'refs/heads/main'
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo bench --workspace 2>&1 | tee bench-results.txt
      - uses: actions/upload-artifact@v4
        with:
          name: bench-results
          path: bench-results.txt
```

### /fixtures/ directory structure

```
fixtures/
├── README.md                  # how to update fixtures
├── update-fixtures.sh         # script to pull from npm
├── radix/
│   ├── button.d.ts
│   ├── dialog.d.ts
│   └── select.d.ts
├── shadcn/
│   ├── button.tsx
│   ├── input.tsx
│   └── badge.tsx
├── mui/
│   ├── Button.d.ts
│   └── TextField.d.ts
├── react-aria/
│   ├── Button.d.ts
│   └── TextField.d.ts
└── panda/
    ├── button.tsx
    └── styled-system/
        ├── css.d.ts
        └── recipes.d.ts
```

### /fixtures/update-fixtures.sh

```bash
#!/bin/bash
# Pull real .d.ts files from npm for fixture testing
# Run: bash fixtures/update-fixtures.sh

set -e
TMPDIR=$(mktemp -d)

pull_dts() {
  local pkg="$1"
  local file="$2"
  local dest="$3"
  npm pack "$pkg" --pack-destination="$TMPDIR" --silent
  TARBALL=$(ls "$TMPDIR"/*.tgz | head -1)
  tar -xzf "$TARBALL" -C "$TMPDIR" "package/$file" 2>/dev/null
  cp "$TMPDIR/package/$file" "$dest"
  rm -rf "$TMPDIR"/*.tgz "$TMPDIR"/package
}

echo "Pulling Radix UI Button..."
pull_dts "@radix-ui/react-button@latest" "dist/index.d.ts" "fixtures/radix/button.d.ts"

echo "Pulling MUI Button..."
pull_dts "@mui/material@latest" "Button/Button.d.ts" "fixtures/mui/Button.d.ts"

echo "Done. Commit the updated fixtures."
```

### /.rustfmt.toml

```toml
edition = "2021"
max_width = 100
use_small_heuristics = "Max"
```

### /.clippy.toml

```toml
msrv = "1.80"
```

## What NOT to Do

- Do not implement any logic — only stubs and structure
- Do not choose HashMap — leave FxHashMap setup for types agent
- Do not add any terminal/display crates to `crates/core/`
- Do not implement NAPI bindings — that is Phase 4a
