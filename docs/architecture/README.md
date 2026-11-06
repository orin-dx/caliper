# Architecture reference

Read this first. Load only the file your task requires — never load all seven at once. This index is short; each topic file runs 1200-3700 words. The goal is progressive loading, not a single monolithic architecture doc.

**These files describe the current system; the source is the ground truth.** When a file here and the code disagree, the code wins and the file is stale: correct it. The records in `.claude/specs/` are historical planning artifacts for work that has shipped; they do not outrank anything here. Each file states its `Source:` — the real `.rs` files it's distilled from — so staleness is always checkable against the code.

## Task → file routing

| Working on... | Load |
| --- | --- |
| File discovery, parallel parse/resolve orchestration, the DTS cache, dependency loading, watch mode, panic containment, plugin hook points | `pipeline-orchestration.md` |
| OXC AST visiting, component detection patterns, JSDoc/default-value extraction, `SourceData` | `extractor-ast-visiting.md` |
| Type resolution, generic substitution, the source-vs-known-pattern precedence rule, the dependency layer's fallbacks, give-up/degrade discipline, cycle detection | `resolver-type-resolution.md` |
| `PropType`/`CollectedType`/`ComponentEntry`/`Diagnostic` shapes, manual serde, `ParsedProp`'s sealed constructor, the msgpack cache wire format | `types-and-output-contract.md` |
| RDT-compat/Storybook/TOON/JSON-Schema output, the `enum` shape rule, `composes`, truncation | `serialization-formats.md` |
| Any `crates/cli` command, exit codes, config loading | `cli-commands.md` |
| `crates/binding` (NAPI/Node), session lifecycle, FFI panic safety | `napi-binding.md` |

## What does NOT live here

- **Source code** — read it directly from the crate. These files summarize invariants, they don't replace reading the function you're about to change.
- **Spec@1 artifacts** (`.claude/specs/`) — planning records of the acceptance criteria for a specific unit of work, written before it shipped. They show what was intended then, not what the system does now; these files are the current description.
- **`docs/edge-cases.md`** — the project's open-gap list. Files here cross-reference it rather than duplicating it; check it directly for current gap status.
- **`docs/adr/`** — hard-to-reverse decisions with their own record format. Files here cite the relevant ADR by number rather than re-explaining the decision.
- **`ARCHITECTURE.md`, `AGENTS.md` files** — human-facing prose reference and contributor rules. Treat as context, not as the invariant source — the files here are the ones checked against real source per subsystem.

## Cross-cutting invariants (span every file — not worth repeating seven times)

- No `unwrap()`/`expect()` outside `#[cfg(test)]` anywhere in `crates/core` or `crates/cli` (verified repeatedly across this project's own audits — holds today).
- `FxHashMap` for internal maps, `BTreeMap` for anything serialized to JSON output (determinism).
- Always emit a `Diagnostic` when degrading — the single most-violated-then-fixed rule in this project's history; every file below documents where this discipline is solid and where residual gaps remain.
- `panic_guard::contain_panic` is the one sanctioned panic-containment boundary (ADR-0005) — see `pipeline-orchestration.md` and `napi-binding.md` for where it's wired in.

## Known doc-hygiene corrections found while writing this layer (2026-08-12)

Writing these seven files against real source turned up several places where existing docs had drifted from the code they describe. Fixed inline in the source docs as part of this pass (see `docs/adr/0002-manual-serialize-for-prop-type.md` for the corrections) — noted here so the correction itself doesn't get lost:

- ADR-0002 claimed `OpaqueReason` hand-writes `Serialize`/is recursive — it doesn't and isn't; only `PropType`/`CollectedType` actually need the manual impl.
- ADR-0002's "worth deleting as a follow-up" note about `#![recursion_limit = "2048"]` — already deleted; the ADR just hadn't been updated to say so.
