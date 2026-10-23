# Serialization formats

**Source:** `crates/core/src/pipeline/rdt.rs` (`serialize`, `docgen_info`, `type_json`, `omits`), `crates/cli/src/commands/extract.rs` (`serialize_rdt`, `serialize_storybook`), `crates/core/src/toon.rs` (whole module), `crates/cli/src/commands/schema.rs` (`schema_value`, `cmd_schema`, its drift test), `crates/core/src/types/output.rs` (`PropType::is_literal_union`), `crates/core/src/resolver/mod.rs` (`ResolvedChain::give_up`, `empty_with_compose`)

This tool's canonical, lossless representation is `ExtractionOutput` — the struct tree in `crates/core/src/types/`, whose JSON writes each distinct prop once and refers to it by id (ADR 0009; see `types-and-output-contract.md`). Everything in this document is a _consumer-facing projection_ of that canonical form: three formats reshape it for specific downstream tools (`--format rdt`, `--format storybook`, `--format toon`), and one (`schema.rs`) describes its shape for external validators. None of the four is the source of truth; `ExtractionOutput` is.

## Where this code lives, and why that's not a AGENTS.md violation

`crates/core/AGENTS.md` says "no terminal/display code in `crates/core`." `serialize_storybook` lives in `crates/cli/src/commands/extract.rs`, not in core — consistent with that rule. But the RDT projection (`pipeline/rdt.rs`) and `toon.rs` (`render_output_toon`, `render_component_toon`, `format_type_compact`) lives in **`crates/core`**, and it is unambiguously a serialization format, not extraction logic.

The line the project actually draws, reverse-engineered from where each format lives: "terminal/display code" means code whose job is to _render for a human looking at a terminal_ (colors, tables, spinners — `crates/cli/src/output.rs`'s comfy-table helpers, `indicatif` progress bars). A _serialization format_ — even a human-readable, LLM-optimized one like TOON — is a consumer-facing data contract, judged by "does another program or prompt consume this," not "is it pretty-printed." TOON is core-side because it's plumbed through `PipelineOptions`/pipeline output the same way canonical JSON is (`caliper_docgen_core::toon::render_output_toon(&output)` at `extract.rs:57`), and because a NAPI/plugin consumer could plausibly want TOON output too, not just the CLI. The RDT shape is core-side for the same reason: the NAPI binding's `sessionDocgenInfo` returns it per component for the Storybook provider (`packages/storybook`), and `--format rdt` (`serialize_rdt`) delegates to the same module. The Storybook format, by contrast, exists _only_ to match another tool's on-disk JSON contract — a CLI-only concern with zero use outside the `--format` flag, so it stays in `crates/cli`.

**Invariant 1:** A new serialization format belongs in `crates/core` only if a non-CLI consumer (NAPI binding, a plugin, a future LSP response) could plausibly want to produce it directly from a `PipelineOptions`/`ExtractionOutput` value. If a format exists solely to match another CLI tool's file-format contract, it belongs in `crates/cli/src/commands/`, not `crates/core`.

## RDT format (`pipeline::rdt`)

Two entry points share one projection: `serialize` (`--format rdt`) applies RDT's default `skipChildrenPropWithoutDoc` (`omits`), and `docgen_info` (the NAPI `sessionDocgenInfo`, feeding the Storybook provider) keeps an undocumented `children`, because Storybook 11's React docgen lists it.

RDT and Storybook output hold only components with `exported: true`, unless `--include-unexported` is passed. react-docgen-typescript documents only a module's exports, so a file-local helper component would otherwise appear in RDT-shaped output where RDT never puts one.

Each RDT component carries `filePath`, the absolute path of its source file, as RDT's `ComponentDoc` does. Storybook format leaves it out.

### The `enum`-shape convention (`type_json`)

react-docgen-typescript encodes a literal union prop type not as an inlined string but as a structured shape renderers pattern-match on:

```rust
if prop_type.is_literal_union() {
    let values: Vec<serde_json::Value> = match prop_type {
        PropType::Union(members) => members.iter().map(|m| serde_json::json!({"value": m.raw_string()})).collect(),
        PropType::LiteralUnion { members, .. } => {
            members.iter().map(|m| serde_json::json!({"value": format!("\"{m}\"")})).collect()
        }
        _ => vec![],
    };
    return serde_json::json!({ "name": "enum", "raw": prop_type.raw_string(), "value": values });
}
serde_json::json!({ "name": prop_type.raw_string() })
```

This exists because Storybook's own docgen addon (a real consumer of RDT's JSON) looks for `type.name === "enum"` as the trigger to render a `<select>` control instead of a free-text field — the single highest-value renderer hook for the most commonly curated kind of prop in any design system (`variant`, `size`, `color`, …).

**Invariant 2:** `rdt::type_json` returns `{"name": "enum", "raw": ..., "value": [...]}` if and only if `prop_type.is_literal_union()` is `true`. Do not special-case `PropType::Union`/`PropType::LiteralUnion` shape-matching directly inside `rdt::type_json` — route every literal-union check through `is_literal_union()`, because that's the single place the 2+ member truthiness rule (below) is enforced.

### The 2+ member truthiness rule

`PropType::is_literal_union()` (`crates/core/src/types/output.rs:365`):

```rust
pub fn is_literal_union(&self) -> bool {
    match self {
        PropType::Union(members) => {
            members.len() >= 2
                && members.iter().all(|m| {
                    matches!(m, PropType::StringLiteral(_) | PropType::NumberLiteral(_) | PropType::BoolLiteral(_))
                })
        }
        PropType::LiteralUnion { members, .. } => members.len() >= 2,
        _ => false,
    }
}
```

**Invariant 3:** A `PropType::Union`/`PropType::LiteralUnion` with **0 or 1** members is never reported as an `enum` shape, regardless of whether every present member is a literal. This was a deliberate correctness fix, not an oversight: a 0- or 1-member "enum" (`{"name": "enum", "value": []}` or a single-entry `<select>`) is actively misleading to a prop-table renderer — it implies a meaningful choice exists when there either is no choice or only one option, which real users would read as a broken/degenerate control rather than a legitimate enum. The known-gaps table in `docs/root-cause-analysis.md`'s standalone-findings section still lists a **zero-member** `LiteralUnion` as a live, reachable (if cosmetic) gap from several construction paths (`known.rs`, `resolver/primitives.rs`, `resolver/alias.rs`, `resolver/mod.rs`, `resolver/template.rs`) — the 2+ guard in `is_literal_union()` is exactly what keeps that gap from surfacing as a broken `enum` shape in RDT output; it degrades to a plain `{"name": raw_string()}` instead.

### `composes` reused for the resolver's give-up cases

RDT natively supports a `composes: string[]` field on each component — "these props come from this named type, listed by reference, not flattened into `props`." This tool's resolver reuses that exact mechanism for a second, distinct purpose: recording every type it gave up trying to resolve. `ResolvedChain::give_up` (`crates/core/src/resolver/mod.rs:380-386`) is "the sanctioned 'give up' entry point for every 'this type couldn't be followed further, stop here' path" — it pushes an optional `Diagnostic` and then calls `empty_with_compose(type_name)`, which sets `composes: vec![type_name]`. Every unresolvable type (a conditional type, a generic pattern the resolver doesn't understand, a cycle) lands in `composes` instead of silently vanishing from `props`.

`rdt::docgen_info` passes `entry.composes` straight through (`"composes": entry.composes`) — this was previously a real bug (see `crates/cli/src/commands/extract.rs`'s `rdt_output_includes_composes` test and its docstring, and `docs/root-cause-analysis.md` cluster 1): the resolver populated `composes` correctly but `serialize_rdt` silently dropped it from the emitted JSON. Fixed; the field now round-trips.

**Invariant 4:** `composes` in the emitted RDT JSON carries two semantically different populations that are _not_ distinguished from each other in the output: (a) types RDT itself would put there — legitimate "props come from elsewhere" composition, and (b) types this resolver gave up resolving. A consumer reading `composes` cannot tell which case produced a given entry without re-running extraction and checking `diagnostics` for a correlated warning. Do not "clean up" `give_up` to stop populating `composes`, and do not add a second field to split the two populations without updating this doc and `resolver-type-resolution.md` — the reuse is intentional (see that file for the resolver-side rationale), not an accident that happens to also serialize correctly.

**Cross-reference:** the resolver-side mechanics of `give_up`/`empty_with_compose`/cycle detection belong to `resolver-type-resolution.md`, not here — this file only documents the serialization-format consequence (the field showing up in RDT JSON), not the resolution logic that populates it.

### `ref`/`key`: deliberate omission, not a bug, not relocated

RDT lists React's `ref` and `key` reconciliation props alongside a component's real props. This tool does not. `docs/benchmarks.md` documents this precisely: of components with any prop-count mismatch against real RDT, "the other 24 (0.9%, spread across 12 of the other 23 components) are **100% `ref`/`key`**... Worth being precise about that one: it's an omission, not a relocation — this tool doesn't surface forwardRef/ref-type metadata anywhere else either (checked `ComponentEntry`'s full field list — no such field exists), so 'we chose not to add it' is accurate, but 'the same info lives elsewhere' isn't."

**Invariant 5:** Neither `ref` nor `key` appears in `props`, in any other `ComponentEntry` field, or in any of the three serialization formats in this document. If a future change adds `ref`-as-prop support (React 19's `function Button({ ref, ...props })` pattern — see `docs/rdt-coverage.md`'s "React 19 `ref`-as-prop" note, already investigated and found to already work structurally when `ref` is an explicit member of the props type), that is a change to what the _resolver_ extracts, not a serialization-format decision — this document's job is only to record that today, none of the three formats invent `ref`/`key` entries that the canonical `ExtractionOutput` doesn't already contain.

### `methods: []` — always empty, and why RDT/Storybook both still carry the field

**Locked design decision:** `rdt::docgen_info` and `serialize_storybook` both hardcode `"methods": []` unconditionally rather than omitting the field or computing it. This tool does not extract class component methods — "this tool doesn't extract class methods, and RDT consumers (Storybook's docgen addon) only ever read it for class components". Since class components are explicitly out of scope for this project (`docs/rdt-coverage.md`'s Component patterns table: `Class component | ❌ | out of scope (modern React only)`), the field is kept present-but-empty rather than dropped, because RDT/Storybook consumers may assume the key always exists on a valid RDT-shaped component entry — dropping it risks a `TypeError` on `.methods.length` in a consumer that doesn't defensively check for the key's presence, whereas an empty array is always safe to iterate.

## Storybook format (`serialize_storybook`)

A narrower sibling of RDT format: same top-level shape (`displayName`, `props`, `description`, `methods: []`), but with two RDT-specific enrichments stripped:

- `type` is always `{ "name": prop_type.raw_string() }` — **no** `enum`-shape special case. `rdt::type_json` is not called here; `prop_type.raw_string()` is used directly.
- `defaultValue` carries only `{"value": ...}`, dropping RDT's `computed` flag.
- `parent` and `composes` are omitted entirely — neither key appears anywhere in `serialize_storybook`'s output.

**Invariant 6:** `serialize_storybook` must never call `rdt::type_json` — if a future edit makes Storybook format enum-aware, that's a deliberate format change requiring conscious sign-off, not an accidental drift from copy-pasting `serialize_rdt`'s type-serialization line. The two functions are intentionally divergent, not one delegating to the other, because Storybook's actual docgen-addon contract genuinely wants a flatter shape than RDT's.

## TOON format (`crates/core/src/toon.rs`)

TOON (Token-Optimized Object Notation) is a compact, indentation-and-CSV-like text encoding for `ExtractionOutput`, designed for LLM prompt/agent-context consumption rather than for a program to parse back into structured data.

### It is deliberately lossy — not meant to round-trip

**Locked design decision:** TOON output is not a serialization format in the reversible sense. It:

- Collapses `PropType` into a single compact string per prop (`format_type_compact`) — e.g. `Array<string>`, `Ref<HTMLButtonElement>`, `handler(MouseEvent)`, `opaque(CustomType)` — discarding structure a JSON consumer would get from the tagged `PropType` enum (no way to tell from the string alone whether `SxProps` came from `PropType::SxProps` vs. a `Named` type happening to render the same text).
- Escapes only for CSV-safety (`escape_toon_val`: commas, newlines, quotes) — not for full round-trip fidelity.
- Truncates long union/intersection/literal-union member lists to a fixed cap, replacing the tail with a `"...(+N)"` marker rather than emitting every member.

There is no `parse_toon` / deserializer anywhere in the codebase, and none is planned — TOON is a one-way export.

**Invariant 7:** No code path may treat TOON output as re-parseable back into `ExtractionOutput` or any `PropType`. If a future feature needs a compact-but-lossless format, that is a new format, not an extension of TOON's contract.

### Truncation with a required indicator — recently made consistent

`truncate_with_indicator(parts: &[String], limit: usize, sep: &str) -> String` (`toon.rs:97-104`) is the single shared helper for "cut a member list down to `limit` items, and if anything was cut, say so":

```rust
fn truncate_with_indicator(parts: &[String], limit: usize, sep: &str) -> String {
    if parts.len() <= limit {
        return parts.join(sep);
    }
    let mut shown: Vec<String> = parts[..limit].to_vec();
    shown.push(format!("...(+{})", parts.len() - limit));
    shown.join(sep)
}
```

Three call sites route through it, each with its own cap: `LiteralUnion` (limit 6, separator `|`), `Union` (limit 4, separator `|`), `Intersection` (limit 4, separator `&`).

This was a real, confirmed bug before the fix — documented in `docs/root-cause-analysis.md` cluster 9 ("TOON truncation indicator dropped in one branch"): `LiteralUnion`'s truncate-with-indicator logic existed but was never factored into a shared helper, so when `Union`/`Intersection` needed the same behavior _in the same function_ (`format_type_compact`), the indicator half was silently dropped — those two branches truncated the member list but said nothing about it, making a 6-member union look identical in TOON output to a genuine 4-member union. The root-cause doc's framing is worth keeping in mind here: "this isn't code aging apart over time; it's proof the codebase has no mechanism that makes 'format a truncated list' a single call site, even within one function." The fix (this shared helper) is exactly the enforcement `crates/core/AGENTS.md`'s design philosophy wants — a single named constructor/helper instead of parallel hand-copied logic.

**Invariant 8:** Every `PropType` variant in `format_type_compact` that renders a bounded-length member list (currently `LiteralUnion`, `Union`, `Intersection`) must call `truncate_with_indicator` rather than hand-rolling its own `.join()`/slice logic. A new variant added later with the same "list of formatted members, potentially long" shape (e.g. if `Tuple` ever grows a compact-member rendering) should route through the same helper by default — don't reintroduce the exact bug this fix closed.

**Invariant 9:** The `"...(+N)"` marker is a required, load-bearing signal, not decorative — `N` must be `parts.len() - limit`, i.e. the exact count of dropped members, not a placeholder or a boolean "more exist" flag. A consumer (LLM agent reading TOON context) uses `N` to judge whether the truncated tail is worth fetching via canonical JSON instead.

### `format_type_compact` is a total function over `PropType` — no fallback arm

Every `PropType` variant has an explicit match arm in `format_type_compact` (`toon.rs:107-150`) — there is no wildcard `_ => ...` catch-all. `PropType::Object(_)` and `PropType::Tuple(_)` delegate to `raw_string()` rather than a bespoke compact form (both can be arbitrarily nested/structured, so a hand-written compact form isn't obviously better than the existing raw renderer). `PropType::Opaque(detail)` renders as `opaque(<raw of the opaque detail>)`.

**Invariant 10:** Adding a new `PropType` variant is a compile error in `format_type_compact` until a match arm is added (Rust's exhaustiveness check enforces this directly — no discipline required, the type system does it). Do not add a wildcard arm to silence that error; picking a deliberate compact representation for each new variant is the point.

## JSON Schema export (`schema.rs`)

### Hand-maintained, not derived

`schema_value()` is a `serde_json::json!()` literal written by hand, describing `ExtractionOutput`'s shape for external tooling (editor integrations, validators) that want a JSON Schema rather than parsing Rust structs. It is explicitly **not** derived from the real structs via `schemars` or similar — `docs/root-cause-analysis.md` cluster 10 documents this as a known, confirmed architectural gap with a proposed (not yet accepted) ADR-0006 to derive it instead.

### The drift-detection test's actual mechanism — targeted, not exhaustive

`schema_covers_every_field_name_the_real_output_serializes` (`schema.rs:117-211`) is the only guard against `schema_value()` silently falling out of sync with the real structs. Read honestly, its actual coverage boundary is narrower than "the schema matches the real output":

1. It builds one hand-constructed `ExtractionOutput` fixture with one component (`Button`), one prop (`variant`), one inheritance layer, one diagnostic, and populated stats.
2. It serializes that fixture to a `serde_json::Value` and collects **field names only** (`obj.keys().cloned()`) from five specific nesting points: the component object itself, the `variant` prop object, the first inheritance-layer object, the first diagnostic object, and the `stats` object.
3. It stringifies the entire `schema_value()` output once (`serde_json::to_string`) and checks that every collected field name appears as a **substring** of that stringified schema (`!schema_str.contains(f.as_str())`).

What this test catches: a field present in the real serialized output at one of those five specific object shapes, but whose name string never appears anywhere in the hand-written schema text — e.g. exactly the pre-fix drift documented in the root-cause doc (`.methods`/`.tags` and 5 of 9 `ExtractionStats` fields, `.line`/`.column`/`.help`).

What this test does **not** catch, honestly stated:

- **Nesting/placement correctness.** Because the check is `schema_str.contains(field_name)` over the _entire flattened schema string_, a field name appearing anywhere in the schema — even under a completely unrelated object — satisfies the check. A field accidentally documented under the wrong parent object would still pass.
- **Type correctness.** The test never checks that `schema_value()`'s declared `"type"` for a field matches the real field's serialized JSON type (string vs. object vs. array, etc.) — only that the name string exists somewhere.
- **Nested object shapes not walked by the fixture.** The fixture only descends into 5 specific paths (top-level component fields, the one prop, the one inheritance entry, the one diagnostic, stats). Fields nested deeper or differently — e.g. inside `notableInherited`'s value shape, or inside a prop's `tags`/`declarations`/`defaultValue` sub-objects — are never extracted from the fixture at all, so drift inside those substructures is invisible to this test.
- **Missing-field-in-real-output direction.** The test only checks "is every real field name present in the schema text," never the reverse ("does the schema claim a field that no longer exists in the real struct"). A schema field that's stale (renamed/removed on the Rust side) would not fail this test as long as no _other_ real field collides with its leftover name.

`docs/root-cause-analysis.md` names this precisely: "the only existing test checks that the hand-written JSON is syntactically valid, never that it matches real serialized output" was the pre-fix state; post-fix, the test is better described as **targeted-but-shallow, not exhaustive** — it closes the specific "a whole field silently missing" failure mode for the object shapes it samples, but does not guarantee full recursive schema/struct parity.

**Invariant 11:** Do not describe `schema_covers_every_field_name_the_real_output_serializes` as "verifying the schema matches the output" in any documentation or PR description — it verifies a narrower, specific property (field-name presence as a substring, at 5 sampled nesting depths) that happens to catch the historically-real drift bugs, not general schema/struct equivalence. If a future change needs stronger guarantees, that's the case for ADR-0006's proposed `schemars`-derived schema, not an argument that the current test is already sufficient.

**Locked design decision (proposed, not yet accepted):** ADR-0006 (drafted in `docs/root-cause-analysis.md`, not yet written as a real `docs/adr/0006-*.md` file per the ADR convention) proposes deriving `schemars::JsonSchema` on the plain output structs and hand-writing only `PropType`/`CollectedType`'s impls (mirroring ADR-0002's precedent of hand-writing `Serialize` for those same two recursive enums, to avoid a `schemars`/`serde` recursion-limit issue). Until that ADR is accepted and implemented, `schema_value()` remains the hand-written source of truth for the exported schema, and the targeted-substring test above remains the only drift guard. Do not silently derive the schema for a subset of fields without either accepting ADR-0006 in full or documenting a narrower interim decision — a half-derived, half-hand-written schema with no test coverage plan would be worse than the current fully-hand-written one.

## Known gaps (cross-referenced, not re-derived)

- **`ref`/`key` omission** — see Invariant 5 above; full quantification in `docs/benchmarks.md`.
- **Zero-member `LiteralUnion`/`Union`, upstream of serialization** — `docs/root-cause-analysis.md`'s standalone-findings table lists a zero-member `LiteralUnion` as reachable from several live construction paths (`known.rs`, `resolver/primitives.rs`, `resolver/alias.rs`, `resolver/mod.rs`, `resolver/template.rs`), cosmetic, not correctness-affecting. At the serialization layer specifically, Invariant 3's 2+ member guard already fully closes the "misleading `enum` shape" failure mode: a 0-member union fails `is_literal_union()`'s `>= 2` check, so `rdt::type_json` falls through to the plain `{"name": raw_string()}` branch, never `{"name": "enum", ...}`. The remaining gap is upstream of this document's scope — a 0-member union still renders as a not-very-useful empty-string raw type — and belongs to `resolver-type-resolution.md`, not this one.
- **JSON Schema drift, pre-fix history** — `.methods`/`.tags`, 5 of 9 `ExtractionStats` fields, `.line`/`.column`/`.help` were all previously undocumented in `schema.rs`; the drift-detection test now catches this specific class of regression (see coverage-boundary discussion above).
- **RDT/Storybook `PropType` kind coverage matrix** — which `PropType` variants actually get exercised by which fixture is maintained in `docs/rdt-coverage.md`'s "PropType kinds" table; this document does not reproduce it. Notably, `void`/`never`/`any` are marked `❌ not covered` there for reasons unrelated to serialization (they're not realistic prop types in strict-mode real-world code), not because any serialization format mishandles them.
