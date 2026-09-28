# Extractor: OXC AST Visiting → SourceData

**Source:** `crates/core/src/extractor/mod.rs`, `component.rs`, `alias.rs`, `interface.rs`, `jsdoc.rs`, `defaults.rs`, `visit.rs`

The extractor is Phase 2a of the pipeline (`OXC parse → SourceData`). It walks one file's AST with OXC's `Visit` trait and produces a `SourceData` — a fully-owned, allocator-free snapshot of everything downstream phases need. It runs per-file, in parallel, with zero cross-file knowledge.

## The output contract: `SourceData`

```rust
pub struct SourceData {
    pub interfaces: FxHashMap<String, CollectedInterface>,      // "path:Name"
    pub type_aliases: FxHashMap<String, CollectedTypeAlias>,    // "path:Name"
    pub type_alias_params: FxHashMap<String, Vec<TypeName>>,    // generic alias type params
    pub interface_type_params: FxHashMap<String, Vec<TypeName>>,// generic interface type params
    pub enums: FxHashMap<String, Vec<EnumEntry>>,
    pub const_arrays: FxHashMap<String, Vec<EnumValue>>,        // resolver-internal only
    pub component_mappings: Vec<ComponentMapping>,              // .tsx only
    pub imports: Vec<ImportBinding>,
    pub exports: Vec<LexedExport>,
    pub diagnostics: Vec<Diagnostic>,
}
```

Everything here is `String`/`Vec`/`FxHashMap` of owned data — never a borrowed AST node. This is the whole point of the module (see Invariant 1). The resolver (`resolve_component()`, called in parallel via rayon) consumes `SourceData` merged across files into `GlobalSourceData`; it never touches OXC types.

Two fields are deliberately _not_ part of the "real" output surface: `const_arrays` exists only so the resolver can expand `(typeof X)[number]` into a literal union — it's never serialized to `ExtractionOutput`. `type_alias_params`/`interface_type_params` exist purely to let the resolver recognize a bare generic-parameter reference (`TData` inside `interface Foo<TData>`) as "expected, not unresolvable" rather than a genuine miss — see `resolver/chain.rs`.

## Invariants

1. **No AST reference outlives `parse_file()`.** The `Allocator` is created locally in `parse_file` (mod.rs:135) and dropped when it returns (mod.rs:169). `SourceDataCollector` never stores a `&'a TSType` or similar — every AST value it needs is converted to an owned `CollectedType`/`String`/span-derived `String` before being stashed in `self.data`. **This is convention, not compiler-enforced.** `SourceDataCollector<'src>` _does_ borrow `source: &'src str` (the raw text, not the AST), which is what makes span-slicing (`self.source[start..end]`) possible without an allocator dependency. Nothing in the type system stops a future contributor from adding a `Vec<&'a TSType>` field to the collector — the rule survives only because every existing extraction path routes through `ts_type_to_collected` (which fully owns its output) before storing anything. A reviewer adding a new extraction path must manually verify this, there's no lint or `Send`/`'static` bound that would catch a violation (the collector isn't required to be `'static` — only `SourceData`, its final output, needs to outlive the allocator).

2. **Comments are cloned out of the arena immediately.** `ret.program.comments` lives in the arena; `parse_file` converts it to `Vec<OwnedComment>` (span offsets + `is_block`, no borrowed text) _before_ `visit_program` runs (mod.rs:160-166). JSDoc lookups then re-slice `self.source` by those saved offsets rather than holding onto the original `Comment` AST nodes.

3. **Component detection follows a fixed do-first-match-wins chain, not a scored/ranked system.** In `visit_variable_declaration` (visit.rs:236-274), for a `.tsx` file with a PascalCase binding: `try_fc_annotation` → `try_forward_ref` → `try_hoc_wrapped` → (if none matched and there's an initializer) `try_rename_identifier_wrapped_component`. First `Some` wins; there is no fallback merge of partial matches from different detectors.

4. **`record_skip` and silence are two deliberately different outcomes, not a spectrum.** `record_skip` (mod.rs:236-247, `DiagnosticCode::SkippedCandidate`, `Info` severity) fires only when a shape is recognizably _close_ to a known pattern but missing/malformed a required piece — e.g. a PascalCase `.tsx` binding whose initializer matches none of the four component detectors (visit.rs:254-263), or a PascalCase function declaration with an untyped first param (visit.rs:314-318). A shape that was never a candidate in the first place (a lowercase binding, a zero-param function — visit.rs's own comment on the zero-param case is explicit about this) produces **no diagnostic at all**. Getting this boundary wrong in either direction is a real regression: too eager and `SkippedCandidate` noise drowns real signal in output; too silent and a shape degrades with zero trace, violating non-negotiable #6.

5. **JSDoc association is single-pass and consumption-tracked, not a fresh lookup per call site.** `find_jsdoc_with_tags` (jsdoc.rs:32-60) binary-searches `self.comments` (pre-sorted by span) for the nearest preceding block comment within `PROXIMITY_THRESHOLD` (120 bytes) and marks its `span_end` in `self.consumed_jsdoc` so no later, unrelated call can claim the same comment. This is why `visit_ts_interface_declaration` explicitly claims the interface's own doc comment (`find_jsdoc_with_tags(node.span.start)`, visit.rs:123) _before_ walking into props — otherwise the interface's first prop (processed next, and physically closer to the comment in some layouts) would steal it via the same proximity match. Getting call order wrong here silently reintroduces the "@deprecated tag bleeds onto siblings" class of bug that `test_deprecated_tag_does_not_bleed_to_sibling_props_without_jsdoc` guards against.

6. **`displayName` renames and `defaultProps` merges are deferred, not applied in-place during traversal.** `try_scan_display_name` only _queues_ a rename into `pending_display_name_renames`; it's applied in `finish()`, after the whole file has been walked (mod.rs:257-268). The reason is explicit in the field's own doc comment: applying immediately would change `component_mappings[i].component_name` out from under any _later_ static-property scan in the same file (another displayName assignment, or `try_scan_default_props`) still looking the mapping up by its original identifier. `try_scan_default_props`, by contrast, _does_ mutate immediately (interface.rs:242-244) — safe only because it merges into `param_defaults` rather than the lookup key itself, so it can't invalidate a later scan's ability to find the mapping.

7. **Aliasing clones, never renames in place.** `try_rename_identifier_wrapped_component` (component.rs:303-326) clones the matched base mapping under the new name and records the base's original name in `aliased_away`, filtered out of the final output in `finish()` (mod.rs:260-263) — filtered by _original_ identifier, before any displayName rename could change what that identifier answers to. Renaming in place instead of cloning would make a second, independent alias to the same base (`const Legacy = Base; const New = Base;` — a real pattern, both bindings genuinely public) unable to find `Base` anymore after the first alias claimed it.

8. **Two independent depth guards exist, deliberately not unified.** `MAX_SOURCE_NESTING_DEPTH = 2000` (mod.rs:40) is a cheap pre-parse linear scan over raw bytes counting bracket nesting (`(`, `{`, `[`) — a proxy for whether `oxc_parser`'s recursive-descent grammar will stack-overflow, checked _before_ the real parser even runs. `MAX_TYPE_COLLECT_DEPTH: u8 = 200` (mod.rs:53) instead guards the extractor's own `ts_type_to_collected_at_depth` recursion, incremented once per AST level. They must stay independent: chained conditional types (`A extends B ? C extends D ? ... : ... : ...`) add one AST level per `? :` with **zero brackets**, so the bracket-based guard systematically undercounts exactly this shape (proven by `deeply_chained_conditional_types_hit_the_depth_guard_not_the_bracket_heuristic`, which constructs 600 chained conditionals that stay under the bracket limit yet trip the depth counter). The resolver has its own separate `MAX_DEPTH: u8 = 20` (`resolver/mod.rs:47`) guarding cross-file type resolution recursion — three different ceilings for three different recursion domains (raw-text pre-parse safety, single-file AST-to-struct conversion, cross-file resolution), each calibrated to its own blast radius, none required to match the others.

9. **Bracket-depth scanning explicitly skips comments and string/template literals.** Not just an optimization — real `.d.ts` files (TypeScript's own `lib.dom.d.ts` included) ship MDN-scraped JSDoc prose containing unmatched brackets (`MISSING: RFC(5646, '...')].`). Without the skip, `depth` would go negative, and a `usize` cast of a negative running value wraps to near-`u64::MAX`, spuriously tripping the nesting guard and silently discarding an entire 1.8MB file with 0 interfaces extracted. `test_nesting_guard_ignores_brackets_inside_comments` guards this regression directly.

10. **Interface/type-alias/enum storage keys are namespace-qualified, and this is load-bearing for lookup, not cosmetic.** `scoped_key()` (mod.rs:249-255) prefixes with `namespace_stack.join(".")` when inside a `namespace X { ... }`. This must exactly match how `ts_type_name_str` renders a `TSTypeName::QualifiedName` reference elsewhere in the same file (`X.Y` — mod.rs:318-326), or a same-file reference to a namespace member can never resolve. Verified by `test_namespace_member_stored_under_qualified_name`.

## Locked design decisions

- **`readonly T[]` and `unique`/`readonly` type operators are peeled transparently, not captured as raw text.** `TSTypeOperatorType::Readonly`/`Unique` unwrap to their operand (mod.rs:512-514) because docgen doesn't track mutability or symbol uniqueness — there's nothing to preserve. Before this, the _entire_ modified type degraded to `CollectedType::Raw`, and downstream heuristics reject any raw string containing a space, so `readonly string[]` (a real `@types/react` `ButtonHTMLAttributes` shape) degraded all the way to `Opaque` even though the element type was fully knowable. `keyof`, by contrast, stays structured (`CollectedType::KeyOf`) because its operand may itself need substitution or resolution later.

- **Every previously-uncovered type-alias body shape gets a `Passthrough` fallback, not a silent drop.** `classify_type_alias`'s catch-all (alias.rs:236) wraps _anything_ `ts_type_to_collected` can already represent structurally — bare function types, inline object literals, arrays, tuples — in `CollectedTypeAlias::Passthrough`. This replaced a `_ => None` that made the entire alias vanish from `data.type_aliases` with zero diagnostic; every reference to it elsewhere in the file would then resolve as unknown. The dedicated arms above the catch-all (`Omit`, `Pick`, `Partial`, `Required`, `Readonly`, `Union`/`LiteralUnion`, `Intersection`) exist specifically for shapes needing extra alias-specific semantics (Omit's key-splitting, discriminated-union detection) that a transparent passthrough can't express — the catch-all is correct precisely because it only ever runs for shapes with no such semantics.

- **`Omit<_, keyof T>` keeps the key-source structured instead of eagerly resolving it.** `collect_omit_keys` (alias.rs:257-262) special-cases a `keyof` operand into `CollectedTypeAlias::Omit::omitted_keys_of: Option<Box<CollectedType>>`, deferred to the resolver, because the excluded key set can't be known until `T` itself resolves — same-file-only information isn't enough in general.

- **Bare inline object/union/intersection types used directly as a props argument (`FC<{ x: string }>`, `forwardRef<Elem, A & B>`) get synthesized into anonymous `__anon_N` aliases** rather than being resolved inline or dropped (mod.rs:678-711). This lets the existing alias-resolution machinery handle them uniformly instead of requiring a second code path for "inline props type" vs. "named props type." The `_ => None` fallback below only fires for genuinely unrecognized shapes now — see edge-cases.md P2 (extractor) for what still falls through it (exotic generic expressions inside `FC<...>`).

- **`unwrap_as_expression` peels `as X` casts before every component-detector call site, uniformly.** Real component libraries (Fluent UI, antd) commonly wrap a `forwardRef`/HOC call or a bare identifier alias in a trailing `as` cast — Fluent UI's own source notes this is required to work around the lack of distributive unions in `@types/react`. Every detector in `component.rs` that pattern-matches a `CallExpression` or bare `Identifier` init goes through this peel first, so the cast itself never has to be special-cased per detector.

- **`try_rename_identifier_wrapped_component` matches two distinct real-world shapes with one function**, not two: a call-wrapped rename (`export let X = someLibraryWrapper(InnerFn) as Y` — Headless UI's `forwardRefWithAs`) and a bare passthrough (`const Button = InternalButton;` — antd's pattern). Both reduce to "an identifier reference to an already-collected mapping, optionally wrapped in a recognized-but-uninterpreted call." Splitting these into separate functions was considered unnecessary since the resolution logic (clone-and-rename, track `aliased_away`) is identical either way.

## Component detection: the pattern chain, explicitly enumerated

| Pattern | Recognizes | File |
| --- | --- | --- |
| 1. FC annotation | `const X: FC\|FunctionComponent\|ComponentType\|VFC\|VoidFunctionComponent\|ForwardRefComponent<Props> = ...` | `component.rs:43-104` |
| 2. `forwardRef` call | `const X = React.forwardRef<Ref, Props>((props, ref) => ...)`, `as`-cast-wrapped | `component.rs:107-152` |
| 3. HOC-wrapped | `const X = memo(function X(props: Props) {...})`, incl. `memo(forwardRef<Ref,Props>(...))` | `component.rs:155-225` |
| 4. Function declaration | `function X(props: Props) { ... }` (top-level, `.tsx` only) | `visit.rs:279-325` |
| 5. `ForwardRefExoticComponent` decl | `declare const X: React.ForwardRefExoticComponent<Props & RefAttributes<E>>` (no initializer — `.d.ts` shape) | `component.rs:230-280` |
| — | Identifier-alias rename (not a distinct detector — a fallback when 1-3 all miss) | `component.rs:303-326` |

**Deliberately not recognized** (per `docs/edge-cases.md` Priority 2, "Extractor"): anonymous default-exported function components (`export default function(props: Props) {}` — `visit_function` requires `func.id`); class components (`class Button extends React.Component<Props>`); class-expression components (`const Button = class extends React.Component<Props> {}`); `Object.assign(Component, { Sub: ... })` compound-component pattern (only `X.Y = ...` static-member assignment is handled — see `interface.rs`'s `static_member_assignment`); `satisfies`-wrapped expressions (no `TSSatisfiesExpression` handling anywhere). All of these are silent misses today, not diagnosed — cross-reference `edge-cases.md` rather than treating this list as exhaustive, since new gaps may have been found since.

## `SkippedCandidate` diagnostic: where it is and isn't wired

Wired in:

- `visit_variable_declaration` — PascalCase `.tsx` binding with an initializer, matching none of patterns 1-3 nor the alias-rename fallback (visit.rs:254-263).
- `visit_function` — PascalCase function declaration with a first param that either has no type annotation at all (visit.rs:314-318) or has one that isn't a recognizable props-type reference (visit.rs:304-312).
- `classify_type_alias`'s `Omit`/`Pick`/`Partial`/`Required`/`Readonly` arms — each malformed-argument early-return (missing type args, wrong arg count, unrecognizable base type) calls `record_skip` before returning `None` (alias.rs, throughout).

Explicitly _not_ wired (by design, per Invariant 4):

- A zero-param PascalCase function declaration (no props type source exists at all — not a malformed candidate, never was one).
- A lowercase-first-letter binding (never PascalCase, never entered the candidate chain).
- A `.tsx` PascalCase binding with **no initializer at all** — deferred to Pattern 5 (ambient `.d.ts`-style declarations) or legitimately not a component.

Per `edge-cases.md` Priority 2, several _other_ extractor silent-drop paths still have no `record_skip` call at all despite being structurally identical to the ones that do: `Object.assign` compound components, computed/numeric/symbol interface property keys, and any `TSType` shape unhandled by `extract_props_arg`/`extract_type_name_from_type`. Do not assume `SkippedCandidate` coverage is complete just because some paths have it.

## JSDoc proximity heuristic: known false-positive risk

`PROXIMITY_THRESHOLD = 120` bytes (jsdoc.rs:33) is a heuristic, not a syntactic guarantee — OXC's AST doesn't attach comments to declarations directly, so proximity-by-byte-offset is the whole mechanism. The threshold is sized to survive "blank lines + a decorator line," per its own comment, but nothing distinguishes a genuine leading JSDoc block from an unrelated `/** ... */` block comment that happens to sit within 120 bytes of a following declaration with no real association (e.g. a stray explanatory comment between two interface members). The `is_block` check filters out `//` line comments but not this case. The consumption-tracking (`consumed_jsdoc`) prevents the same comment being double-claimed, which closes the specific sibling-bleed bug covered by `test_deprecated_tag_does_not_bleed_to_sibling_props_without_jsdoc`, but does not address a block comment being wrongly claimed once, by the wrong element, in the first place.

## Known gaps not otherwise covered above

See `docs/edge-cases.md` for the full audit; the extractor-relevant subset beyond component-detection gaps:

- `classify_type_alias`'s `Omit`/`Pick`/`Partial`/`Required`/`Readonly` arms drop the _entire_ alias on malformed arguments rather than degrading partially (alias.rs) — now diagnosed via `record_skip`, but the alias is still fully absent from `data.type_aliases`.
- Any `TSType` variant unhandled by `extract_props_arg`/`extract_type_name_from_type` silently produces no `ComponentMapping` at all, with no `SkippedCandidate` trace (unlike the visit.rs call sites).
- Deep conditional-type chains are covered by the depth guard (Invariant 8) but P1-8 in `edge-cases.md` flags this as previously under-tested — now covered by `deeply_chained_conditional_types_hit_the_depth_guard_not_the_bracket_heuristic`.
