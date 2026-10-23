# Quality & Architecture Refactor Plan

**Status:** Planning — not yet started  
**Prerequisites:** All existing 90 tests must stay green throughout  
**Goal:** A codebase that passes open-source scrutiny, not just CI

---

## Honest Diagnosis

Before prescribing solutions, the actual problems:

| File           | Lines | Fns | Problem                                        |
| -------------- | ----- | --- | ---------------------------------------------- |
| `resolver.rs`  | 2,482 | 55  | Six unrelated concerns in one file             |
| `extractor.rs` | 1,867 | 62  | One 1,800-line `impl` block                    |
| `types.rs`     | 1,305 | 20  | Two abstraction layers + diagnostics all mixed |
| `cli/main.rs`  | 760   | 14  | Five commands + config + output in `main.rs`   |

Beyond size, the specific code-level problems:

1. **8-parameter functions** — `resolve_props_chain(type_name, type_args, consuming_file, mapping, ctx, visited, depth, diagnostics)`. Error-prone at call sites, impossible to add parameters without touching every caller.

2. **`FxHashSet<String>` for the visited set** — heap allocates a `String` per visited type name. `CompactString` is already a workspace dep; most type names fit inline (≤24 bytes).

3. **Silent IO failures** — `fs::read_to_string(path).unwrap_or_default()` makes a permission error indistinguishable from an empty file. Users get 0 components, no diagnostic.

4. **Hand-rolled serde for `PropType` and `CollectedType`** — written to work around a 19-minute compile time caused by serde derive recursion. Goes through `serde_json::Value` as an intermediate, allocating a heap value per prop during every serialization. Root cause: recursive enum variants weren't boxed. Fix is `Box<PropType>` on the `Array` variant (already done for `CollectedType`) and re-enabling derive.

5. **`strip_json_comments` lives in `resolver.rs`** — a JSON utility function in the type resolver. Should be a crate or a `util` module.

6. **`scoped_key` as raw `String`** — map keys are `format!("{}:{}", file_path, name)` constructed in multiple places. No type safety, no canonical constructor, compared by string equality everywhere.

7. **Everything is `pub mod`** — `lib.rs` re-exports all eight modules. Consumers of the library crate can reach `extractor::SourceDataCollector`, `resolver::resolve_named`, `known::resolve_known` — all implementation details. The real public API is: `pipeline::{extract, PipelineOptions, WatchSession}`, `types::{ExtractionOutput, Diagnostic, DiagnosticSeverity}`, `react_types::{REACT_18, REACT_19}`.

---

## Principles

**The module split is the enforcement mechanism, not a tool on top.** Once `resolver.rs` becomes `resolver/mod.rs + chain.rs + alias.rs + import.rs + html.rs`, you cannot recreate a 2,500-line resolver file without actively fighting the structure. No shell script needed.

**Structural and behavioral changes together.** Moving code between files without fixing the design problems just moves the mess around. The behavioral changes (ResolveState, ScopedKey, error propagation, serde) are equally important and should happen alongside the structural split, not after.

**Tests are the safety net.** The 90 existing tests define correct behavior. Every refactor step must keep them green. The `insta` snapshot tests (already a workspace dep) should be added before the refactor starts to lock in output format.

**Clippy at the right thresholds catches regressions.** Configure `.clippy.toml` with `too-many-arguments-threshold = 5` and `too-many-lines-threshold = 80` so that the violations we fix today can't creep back in.

---

## Module Structure (Target)

### `crates/core/src/`

```
lib.rs                  — public re-exports only (see Visibility section)

types/
  mod.rs                — re-exports for types consumers need
  collected.rs          — CollectedType, SourceData, RawProp, CollectedInterface,
                          CollectedTypeAlias, ImportBinding, LexedExport,
                          ComponentMapping, ExtendsRef, DefaultSource, RawDefault
  output.rs             — PropType, ComponentEntry, ExtractionOutput,
                          InheritedLayer, ParsedProp, ObjectField, DefaultValue,
                          PropParent, EnumEntry, EnumValue, ExtractionStats
  diagnostic.rs         — Diagnostic, DiagnosticSeverity, DiagnosticCode, OpaqueReason
  global.rs             — GlobalSourceData, ScopedKey (see below)

extractor/
  mod.rs                — pub fn parse_file() entry point; SourceDataCollector struct
  component.rs          — try_fc_annotation, try_forward_ref, try_hoc_wrapped,
                          try_forward_ref_exotic_decl (component detection patterns)
  interface.rs          — collect_interface, collect_raw_props, collect_object_fields
  alias.rs              — classify_type_alias (Omit/Pick/Passthrough/Union/Intersection)
  jsdoc.rs              — find_jsdoc, extract_jsdoc_tags, parse_jsdoc_text
  visit.rs              — impl Visit for SourceDataCollector (AST walker entry points)
  defaults.rs           — extract_param_defaults, destructure default extraction

resolver/
  mod.rs                — pub fn resolve_component(); ResolutionContext
  state.rs              — ResolveState (see Behavioral section)
  chain.rs              — resolve_props_chain, resolve_interface_chain
  extends.rs            — resolve_extends_ref
  alias.rs              — resolve_type_alias_chain, resolve_base_as_chain,
                          resolve_union_alias, resolve_type_alias_type
  primitives.rs         — resolve_union, resolve_intersection, resolve_indexed_access
  collected.rs          — resolve_collected_type (CollectedType → PropType dispatch)
  named.rs              — resolve_named (the main named-type lookup)
  template.rs           — resolve_template_literal, try_expand_template_literal,
                          resolve_named_to_string_literals
  func.rs               — resolve_function_type, resolve_typeof
  import.rs             — resolve_to_canonical, resolve_import_specifier
  html.rs               — infer_html_attr_prop_type, capitalize_element
  react.rs              — react_type_to_prop_type, resolve_react_types_file

pipeline/
  mod.rs                — pub fn extract(); pub struct PipelineOptions
  discover.rs           — discover_files (walk + filter)
  resolve.rs            — multi-pass resolution loop (including cross-package)
  watch.rs              — WatchSession

known.rs                — (unchanged structure; single match-on-str dispatch is idiomatic
                          and LLVM optimizes it well for <150 arms. Do not convert to phf.)
import_map.rs           — (unchanged)
cache.rs                — (unchanged)
react_types.rs          — (unchanged)
```

### `crates/cli/src/`

```
main.rs                 — Cli struct, Command enum, main(), init_tracing()
config.rs               — load_config_file, try_load_config, build_options
output.rs               — print_summary, print_diagnostics, formatting helpers
commands/
  mod.rs
  extract.rs            — cmd_extract, serialize_rdt, serialize_storybook
  watch.rs              — cmd_watch
  inspect.rs            — cmd_inspect
  check.rs              — cmd_check
  completions.rs        — cmd_completions
```

---

## Behavioral Changes

These are not cosmetic. Each one changes what the code does.

### 1. `ResolveState` — the most load-bearing change

Replaces the `visited`, `depth`, and `diagnostics` parameters threaded through every resolver function. Also adds `needed_specifiers` for cross-package resolution (see below).

```rust
// resolver/state.rs
pub(crate) struct ResolveState {
    visited: FxHashSet<CompactString>,      // was FxHashSet<String>
    pub diagnostics: Vec<Diagnostic>,
    depth: u8,
    /// Packages we needed but weren't in GlobalSourceData.
    /// Populated during resolution; consumed by the pipeline for cross-pkg loading.
    pub needed_specifiers: Vec<NeededSpecifier>,
}

pub(crate) struct NeededSpecifier {
    pub package: String,            // e.g. "@radix-ui/react-primitive"
    pub from_file: Utf8PathBuf,     // the file that imported it
}

impl ResolveState {
    pub fn new() -> Self { ... }

    /// Returns false if already visited (cycle detected).
    pub fn visit(&mut self, key: &str) -> bool {
        self.visited.insert(CompactString::from(key))
    }

    /// Returns None if depth limit exceeded (emits diagnostic automatically).
    pub fn descend(&mut self) -> Option<DepthGuard<'_>> { ... }

    pub fn note_needed(&mut self, package: String, from_file: Utf8PathBuf) {
        self.needed_specifiers.push(NeededSpecifier { package, from_file });
    }
}
```

Every resolver function signature goes from 8 parameters to 4:

```rust
// Before
fn resolve_props_chain(
    type_name: &str, type_args: &[String], consuming_file: &Utf8Path,
    mapping: &ComponentMapping, ctx: &ResolutionContext,
    visited: &mut FxHashSet<String>, depth: u8, diagnostics: &mut Vec<Diagnostic>,
) -> ResolvedChain

// After
fn resolve_props_chain(
    type_name: &str, type_args: &[String],
    consuming_file: &Utf8Path, ctx: &ResolutionContext,
    state: &mut ResolveState,
) -> ResolvedChain
```

### 2. `ScopedKey` newtype

Map keys are currently `format!("{}:{}", file_path, name)` strings constructed in 6+ different places. A misformatted key silently produces a miss instead of an error.

```rust
// types/global.rs
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct ScopedKey(CompactString);

impl ScopedKey {
    pub fn new(file: &Utf8Path, name: &str) -> Self {
        let mut s = CompactString::with_capacity(file.as_str().len() + 1 + name.len());
        s.push_str(file.as_str());
        s.push(':');
        s.push_str(name);
        Self(s)
    }
    pub fn as_str(&self) -> &str { &self.0 }
}
```

`GlobalSourceData` maps become `FxHashMap<ScopedKey, ...>` instead of `FxHashMap<String, ...>`. The compiler then enforces correct key construction everywhere.

### 3. Proper IO error propagation

```rust
// Before — permission error looks like empty file
let source = std::fs::read_to_string(path).unwrap_or_default();

// After — IO failures become diagnostics, not silent empties
match std::fs::read_to_string(path) {
    Ok(source) => source,
    Err(e) => {
        diagnostics.push(Diagnostic::io_error(path, e));
        continue;
    }
}
```

### 4. `PropType` serde — remove the intermediate `serde_json::Value`

The hand-rolled impl was written because serde derive hit the recursion limit. The actual fix is `Box<PropType>` on recursive single-element variants. `Array` already uses `Box<PropType>`. The recursion comes from `Union(Vec<PropType>)` and `Intersection(Vec<PropType>)` — but `Vec<T>` doesn't recurse the serde macro (it uses a generic impl). The recursion comes from `Array(Box<PropType>)` being visited, which leads back to `PropType`.

Profile this: try removing the manual impl and using `#[derive(Serialize, Deserialize)]` with the existing `Box<PropType>` on `Array`. If the recursion limit is still hit, increase `recursion_limit` — it's already set to 2048. The 19-minute compile was a serde 1.x codegen bug that is fixed in recent versions.

If derive works: the intermediate `serde_json::Value` allocation goes away, the hand-rolled 200+ line impl goes away, and adding new variants to `PropType` stops requiring manual serde code.

### 5. `FxHashSet<CompactString>` for visited

One-line change, uses existing dep, eliminates heap allocation for the common case (most type names ≤24 bytes).

### 6. Replace `strip_json_comments` with a crate

```toml
# Cargo.toml
json-strip-comments = "1" # or serde_json5 = "0.1"
```

Remove the 20-line hand-written function from `resolver.rs`. This is in the wrong file regardless.

---

## Visibility Model

**Public API surface** (what `lib.rs` exposes as `pub`):

```rust
// lib.rs
pub mod pipeline;           // extract(), PipelineOptions, WatchSession
pub mod types {             // re-export only output types
    pub use crate::types_impl::output::*;
    pub use crate::types_impl::diagnostic::*;
}
pub mod react_types;        // REACT_18, REACT_19

// Everything else is pub(crate):
pub(crate) mod extractor;
pub(crate) mod resolver;
pub(crate) mod known;
pub(crate) mod cache;
pub(crate) mod import_map;
pub(crate) mod types_impl;  // renamed from types to avoid collision
```

**Within modules**, functions are `pub(crate)` if needed across modules, private otherwise. Nothing in `resolver/` or `extractor/` is `pub` unless it's `resolve_component` or `parse_file`.

`cargo-semver-checks` in CI enforces this — if we accidentally expose an internal type in a future release, the check fails.

---

## Cross-Package Resolution

Design established in prior sessions. Captures the key decisions here for context persistence.

**Mechanism:** Import-guided, demand-driven, multi-pass.

When `resolve_named` cannot find a type and can identify which import binding it came from (e.g., `Primitive` from `import { Primitive } from '@radix-ui/react-primitive'`), it calls `state.note_needed(package, from_file)` instead of (or in addition to) emitting a warning.

After a resolution pass, `pipeline/resolve.rs` drains `state.needed_specifiers`, uses the already-present `ctx.oxc_resolver` to find each package's `.d.ts` entry point, parses them in parallel (same rayon pattern as Phase 1), merges into `GlobalSourceData` via `rcu`, and runs another resolution pass. Stops when frontier is empty or depth > 3 hops.

**Nothing new to add to the workspace.** `oxc_resolver` is already wired into `ResolutionContext`. `DtsCache` already caches `.d.ts` parse results. `GlobalSourceData::merge()` already accepts new files. `parse_file()` already handles `.d.ts`. The loop in `pipeline/resolve.rs` is ~40 lines of wiring, not new machinery.

**Stopping conditions:**

- Frontier is empty (all needed types resolved)
- Package already in `GlobalSourceData` (cycle / duplicate)
- Depth > 3 hops from original source files
- Package count added > 20 (safety valve)
- Package is in the known-React-types set (handled by `known.rs`, no file needed)

**Opt-in config:**

```json
{ "followDeps": true }   // default: true when node_modules/ is accessible
{ "followDeps": false }  // explicit opt-out, strict single-package mode
{ "followDeps": ["@radix-ui/*", "@floating-ui/react"] }  // allowlist
```

---

## Tooling

What actually matters for THIS codebase:

**`.clippy.toml` — tighten thresholds to catch regressions:**

```toml
msrv = "1.94"
too-many-arguments-threshold = 5
too-many-lines-threshold = 80
cognitive-complexity-threshold = 20
```

**`lib.rs` — promote key lints:**

```rust
#![warn(clippy::pedantic)]
#![warn(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]  // ResolvedChain in resolver/ is fine
#![allow(clippy::must_use_candidate)]       // too noisy for internal fns
#![allow(clippy::missing_errors_doc)]       // internal fns don't need this
```

**`insta` snapshot tests** (already in workspace) — add before refactor starts. Lock the output format for every fixture. If any refactor step changes JSON output, the snapshot diff makes it visible immediately.

**`divan` benchmarks** (already in workspace) — add baseline benchmarks for `parse_file`, `resolve_component`, and `extract` on representative fixtures before behavioral changes. After: verify no regression.

**`proptest`** — property-based testing for the parser. Generate random valid-ish TypeScript strings and assert `parse_file` never panics. This is table stakes for a parser tool heading toward open source.

**`cargo-semver-checks`** in CI — once the public API is locked down (visibility cleanup done), add to CI. Prevents accidental API surface expansion in releases.

**`cargo-deny`** — add `deny.toml` for license policy (MIT/Apache-2.0 only) and to ban duplicate major versions of OXC crates (they must all be 0.135.x). OXC version skew causes subtle bugs.

**What we are NOT doing:**

- `cargo-modules` — visualization only, doesn't enforce anything
- `phf` for `known.rs` — LLVM optimizes our 50-arm match; phf adds overhead
- Custom shell scripts for file length — the module split is the enforcement
- `cargo-machete`, `cargo-audit`, `cargo-bloat` — fine tools but none of them fix any of the identified problems

---

## Implementation Sequence

Each phase keeps tests green. No skipping.

### R1 — Add snapshot tests (before touching anything)

Add `insta` snapshot tests for every fixture's extraction output. These are the safety net for the entire refactor. ~1 day.

### R2 — `types/` split

Split `types.rs` into `types/collected.rs`, `types/output.rs`, `types/diagnostic.rs`, `types/global.rs`. Introduce `ScopedKey`. Update all imports across the codebase. Pure structural — zero behavior change. Tests must stay green. ~1 day.

### R3 — `ResolveState`

Introduce `resolver/state.rs`. Thread `ResolveState` through `resolve_component` and all callee functions, replacing `visited`, `depth`, `diagnostics` parameters. Change `FxHashSet<String>` to `FxHashSet<CompactString>` in the same pass. Tests must stay green. ~1 day.

### R4 — `resolver/` module split

With `ResolveState` in place and cleaner function signatures, split `resolver.rs` into the module directory. Move `strip_json_comments` out (use a crate). Tests must stay green. ~1 day.

### R5 — `extractor/` module split

Split `extractor.rs` into the module directory. Tests must stay green. ~half day.

### R6 — `pipeline/` split + IO error propagation

Split `pipeline.rs`. Fix the `unwrap_or_default()` IO failures to emit diagnostics. ~half day.

### R7 — CLI split

Split `cli/main.rs` into commands/. No behavioral change. ~half day.

### R8 — Visibility cleanup

Change internal modules to `pub(crate)`. Establish the real public API in `lib.rs`. Add `cargo-semver-checks` to CI. ~half day.

### R9 — `PropType` serde

Attempt `#[derive(Serialize, Deserialize)]` — if it works cleanly, remove the hand-rolled impl. If not, document why and move on. ~half day, possibly faster.

### R10 — Cross-package resolution

With clean module structure and `ResolveState.needed_specifiers` in place, implement the multi-pass loop in `pipeline/resolve.rs`. ~1-2 days.

### R11 — Tooling

`.clippy.toml` thresholds, `cargo-deny`, `cargo-semver-checks`, `proptest` for the parser. ~half day.

---

## What Does NOT Change

- Data flow: `OXC parse → SourceData → GlobalSourceData → ComponentEntry → ExtractionOutput`
- All 90 existing unit tests
- The `known.rs` dispatch pattern (it's idiomatic and fast)
- The `DtsCache` implementation
- The `import_map.rs` implementation
- The `react_types.rs` data tables
- The `ArcSwap` + rayon parallel resolution (already correct)
- The NAPI and CLI public interfaces
- The output JSON format (guarded by snapshot tests)
