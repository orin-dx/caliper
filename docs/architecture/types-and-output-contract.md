# Types & the Output Contract

**Source:** `crates/core/src/types/output.rs`, `crates/core/src/types/diagnostic.rs`, `crates/core/src/types/collected.rs`, `crates/core/src/types/global.rs`, `crates/core/src/react_types.rs`

This is the data shared between every phase of the pipeline (`OXC parse → SourceData (extractor) → GlobalSourceData (pipeline) → ComponentEntry (resolver) → ExtractionOutput`). These types _are_ the contract: extractor and resolver code on either side of a phase boundary only agree on behavior because they agree on these shapes.

## The two-representation split

There are two recursive "type" enums, one per side of the resolver boundary:

- `CollectedType` (`types/collected.rs`) — the extractor's raw, unresolved AST-level representation. Still has TypeScript-shaped variants: `Conditional`, `Mapped`, `TypeOf`, `KeyOf`, `IndexedAccess`, `TemplateLiteral`, `Raw`.
- `PropType` (`types/output.rs`) — the resolver's fully-resolved semantic output. TypeScript-shaped ambiguity has been resolved away or converted to `PropType::Opaque` with a reason.

`resolver/` pattern-matches `CollectedType` to produce `PropType`; nothing downstream of the resolver ever sees a `CollectedType` again.

```rust
pub enum CollectedType {
    String, Number, Boolean, Null, Undefined, Any, Never, Unknown, Void, BigInt, Symbol,
    StringLiteral(CompactString), NumberLiteral(f64), BoolLiteral(bool),
    Union(Vec<CollectedType>), Intersection(Vec<CollectedType>), Array(Box<CollectedType>),
    Tuple(Vec<CollectedType>), Object(Vec<CollectedObjectField>),
    Named { name: CompactString, args: Vec<CollectedType> },
    TypeOf(CompactString),
    KeyOf(Box<CollectedType>),
    AtFile { file: Utf8PathBuf, inner: Box<CollectedType> },
    IndexedAccess { obj: Box<CollectedType>, key: Box<CollectedType> },
    TemplateLiteral(Vec<CollectedType>),
    Function { params: Vec<CollectedType>, param_names: Vec<Option<CompactString>>, return_type: Box<CollectedType> },
    Conditional { check: Box<CollectedType>, extends_type: Box<CollectedType>, true_type: Box<CollectedType>, false_type: Box<CollectedType> },
    Mapped { key_type: Box<CollectedType>, value_type: Box<CollectedType> },
    Raw(std::string::String),
}
```

```rust
pub enum PropType {
    String, Number, Boolean, Null, Undefined, Any, Never, Unknown, Void,
    StringLiteral(String), NumberLiteral(f64), BoolLiteral(bool),
    Union(Vec<PropType>), Intersection(Vec<PropType>), Array(Box<PropType>),
    Tuple(Vec<PropType>), Object(Vec<ObjectField>),
    Named { name: TypeName, args: Vec<PropType> },
    ReactNode, CssProperties,
    EventHandler { event_type: String, param_name: Option<String> },
    Ref { element: Option<String> },
    ElementType,
    HtmlAttributes { element: String, omitted: Vec<String> },
    LiteralUnion { members: Vec<String>, has_default: bool },
    SxProps,
    Opaque(OpaqueDetail),
}
```

Note what disappeared crossing the boundary: `BigInt`, `Symbol`, `TypeOf`, `KeyOf`, `AtFile`, `IndexedAccess`, `TemplateLiteral`, `Function`, `Conditional`, `Mapped`, `Raw` are all gone from `PropType`. Each either resolves into a concrete `PropType` variant (`Function` with the right shape becomes `EventHandler`) or terminates in `PropType::Opaque` with an `OpaqueReason` explaining exactly which unresolvable TypeScript construct it was (`ConditionalType`, `MappedType`, `IndexedAccess`, `TemplateLiteral`, ...). This is the resolver's entire job, structurally: eliminate every "can't be sure yet" variant.

## LOCKED DESIGN DECISION: manual Serialize/Deserialize on the recursive enums (ADR-0002)

**Source:** `docs/adr/0002-manual-serialize-for-prop-type.md`

`PropType` and `CollectedType` do **not** derive `Serialize`/`Deserialize`. Each hand-writes the round-trip through a private `to_json_value`/`from_json_value` pair that builds/reads a `serde_json::Value` directly (`output.rs:436-715`, `collected.rs:209-461`), then wires that into the `serde::Serialize`/`serde::Deserialize` traits by hand.

**Why:** these are recursive enums (`Union(Vec<Self>)`, `Object(Vec<Field>)` where `Field` contains `Self`, `Array(Box<Self>)`, ...). A derived, tagged (`#[serde(tag = "kind")]`) implementation on a recursive enum needed `#![recursion_limit = "2048"]` to compile — each nesting level wraps the generated (de)serializer in another codegen layer, and a real component prop tree blows past the default limit (128). Manual impls sidestep the codegen recursion entirely; they're just recursive Rust functions, which the compiler doesn't limit the same way.

**Do not "simplify" this back to `#[derive(Serialize, Deserialize)]`.** It looks removable — it's a lot of boilerplate for something serde could theoretically generate — but doing so reintroduces the recursion-limit dependency, and worse, silently drops the manual NaN/Infinity handling described below.

**Correction to the ADR itself, verified against current code (2026-08):** ADR-0002 states "`PropType`, `CollectedType`, and `OpaqueReason` don't derive Serialize/Deserialize." This is stale for `OpaqueReason`. The current source (`output.rs:743-745`) shows:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpaqueReason {
    ConditionalType, MappedType, ModuleAugmentation,
    RuntimeDependent { function_name: String },
    UnresolvableImport { specifier: String },
    PandaCodegenMissing, DepthExceeded,
    IndexedAccess { expression: String },
    TemplateLiteral { expression: String },
    MultiParamFunction, UnsupportedExpression,
}
```

`OpaqueReason` derives normally. It isn't recursive — every variant's payload is a `String` or a unit — so it never hit the recursion-limit problem the ADR is about. The _serialization of an `OpaqueReason` value_ is still hand-assembled inside `PropType::to_tagged_value`'s `Opaque` arm (`output.rs:538-567`, matching each reason to its own `serde_json::json!{...}` shape) and hand-parsed in `from_tagged_value` (`output.rs:687-711`) — but that's because it's embedded inside `PropType`'s own manual impl, not because `OpaqueReason` itself needs one. This is a small, real documentation bug in ADR-0002 worth a one-line fix, independent of anything else in this file.

Also worth noting as **resolved, not a gap:** the ADR flagged `#![recursion_limit = "2048"]` in `lib.rs` as "left over from the old derive-based design... worth deleting as a follow-up." As of the current source, `crates/core/src/lib.rs` has no `recursion_limit` attribute at all — that follow-up has already happened. The ADR's "Consequences" section is stale on this point too.

### The "no wildcard matches" convention

**Invariant 1:** Every match on `PropType` or `CollectedType` inside the manual serde impls, and by extension idiomatically everywhere else in the resolver, must be exhaustive — no `_ => ...` fallthrough arm.

**Why it's load-bearing:** a derived serde impl gives you a compiler error the moment you add a new enum variant and forget to handle it in serialization. A hand-written match loses that safety net _unless every match stays exhaustive_ — a wildcard arm would silently absorb a new variant into whatever the wildcard does (often `Opaque` or a panic), and nothing would tell you serialization is now lossy for that variant. `to_tagged_value` (output.rs:451-568) and `from_tagged_value` (output.rs:576-712), and `to_json_value`/`from_json_value` in `collected.rs`, are all written this way. If you add a `PropType` variant, the compiler _will_ fail the exhaustiveness check in `to_tagged_value` and force you to handle it — but only because nobody put a wildcard there. Adding one defeats the entire safety mechanism this ADR relies on.

### NaN/Infinity as a wire-format special case

**Invariant 2:** `PropType::NumberLiteral(f64)` and `CollectedType::NumberLiteral(f64)` encode non-finite values (`NaN`, `Infinity`, `-Infinity`) as JSON strings (`"NaN"`, `"Infinity"`, `"-Infinity"`), not as JSON numbers, and the read side must check for a string before falling back to `as_f64()`.

**Why:** `serde_json::Number` (and JSON itself) cannot represent non-finite floats — `serde_json::json!(f64::NAN)` silently becomes `null`. Without the string-tag workaround, a `NumberLiteral(NaN)` would round-trip as `0.0` (`output.rs` comment at line 474-476; `collected.rs` at 226-230) or, in `CollectedType`'s case, fail every match arm in `from_json_value` and fall through to `Raw`. Both files have explicit round-trip tests for this (`number_literal_roundtrip_tests` in `output.rs`, the NaN/Infinity/-Infinity tests in `collected.rs`'s `mod tests`) — treat those as regression guards, not optional coverage.

## LOCKED DESIGN DECISION: `OpaqueDetail::new` vs `OpaqueDetail::give_up`

**Source:** `crates/core/src/types/output.rs:316-357`, cross-referenced against call sites in `crates/core/src/known.rs` and `crates/core/src/resolver/*.rs`

`PropType::Opaque`'s payload, `OpaqueDetail { raw: String, reason: OpaqueReason }`, has private fields — nothing outside `types/output.rs` can construct one by struct literal. There are exactly two ways to build a `PropType::Opaque` value:

```rust
impl OpaqueDetail {
    pub(crate) fn new(raw: impl Into<String>, reason: OpaqueReason) -> PropType { ... }

    pub(crate) fn give_up(
        state: &mut ResolveState,
        raw: impl Into<String>,
        reason: OpaqueReason,
        diagnostic: Diagnostic,
    ) -> PropType {
        state.diagnostics.push(diagnostic);
        Self::new(raw, reason)
    }
}
```

**Invariant 3:** every resolver call site that degrades a type to `Opaque` during real resolution work must go through `give_up`, never `new`, because `give_up` is the only path that guarantees a `Diagnostic` gets pushed onto `ResolveState` alongside the degraded value. This directly enforces AGENTS.md non-negotiable #6 ("Always emit `Diagnostic` when degrading — never fail silently") at the type level: it is not possible to produce an `Opaque` from inside the resolver proper without also explaining why in the diagnostics stream, because the constructor that skips that step is a different, narrower-purpose function.

**Who actually calls which, verified against current source:**

- `resolver/collected.rs`, `resolver/primitives.rs`, `resolver/template.rs`, `resolver/named.rs`, `resolver/func.rs` — every one of these resolver-internal degradation sites calls `OpaqueDetail::give_up`, each with its own specific `OpaqueReason` (`DepthExceeded`, `MappedType`, `ConditionalType`, `UnsupportedExpression`, `IndexedAccess`, `TemplateLiteral`, `MultiParamFunction`) and a diagnostic message tailored to that reason.
- `known.rs` — every call is `OpaqueDetail::new`, for the documented exception: `known.rs` has no `ResolveState` of its own to push a diagnostic onto (it's a lookup table, not a resolution pass), so it pushes its diagnostic separately through `push_known_opaque_diagnostic` (`known.rs:31`) at the call site that invokes the lookup, rather than inline in the constructor.
- `output.rs`'s own `from_tagged_value` — deserialization uses `OpaqueDetail::new` for both the "opaque" JSON tag and the catch-all unknown-`kind` fallback (line 712). This is deserialization, not resolution — there is no `ResolveState` in scope during deserialization, and re-diagnosing a value that's simply being read back off disk/wire would be meaningless (the diagnostic, if any, was already emitted and persisted when the value was first produced).

Read `resolver-type-resolution.md` for the call-site-level reasoning about _why_ each of those resolver locations gives up; this file's job is only the type-level contract: `Opaque` values arising from live resolution are diagnostic-carrying by construction, and the type system (via field privacy + two narrowly-scoped constructors) is what makes that true rather than a convention someone has to remember.

## LOCKED DESIGN DECISION: `ParsedProp`'s sealed-field constructor

**Source:** `crates/core/src/types/output.rs:118-210`

```rust
pub struct ParsedProp {
    pub name: String,
    #[serde(rename = "type")]
    pub prop_type: PropType,
    pub required: bool,
    pub default_value: Option<DefaultValue>,
    pub description: String,
    pub tags: BTreeMap<String, String>,
    pub parent: Option<PropParent>,
    pub declarations: Vec<PropParent>,
    #[serde(skip)]
    _seal: Seal,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Seal;
```

Every field except `_seal` is `pub` — `crates/cli` reads `ParsedProp` fields directly across the crate boundary (table rendering, TOON output, etc.), so `pub(crate)` was rejected as the fix; it would have blocked exactly the consumer this type exists to serve. Instead, one private, zero-sized `Seal` field is appended. Because Rust struct-literal syntax requires every field to be named (there is no default-fill for a private field from outside the defining module), a bare `ParsedProp { name, prop_type, required, ... }` literal is a compile error everywhere except inside `types/output.rs` itself — including other modules in the _same crate_. The only way to construct a `ParsedProp` from outside this module is `ParsedProp::new(...)`.

**Invariant 4:** a `ParsedProp` can never exist with `required: true` and `default_value: Some(_)` simultaneously. `ParsedProp::new` (output.rs:159-171) enforces this by construction:

```rust
let required = if default_value.is_some() { false } else { required };
```

**Why this specific invariant is worth sealing the type over:** the contradictory state is easy to produce _upstream_ by accident. A destructured parameter like `({ size = 'md' }: Props)` can have a type annotation that marks `size` as required (no `?` in the interface) while also carrying a default expression in the destructuring pattern — two independent extractor passes (interface parsing vs. destructuring-default extraction) can each report a technically-correct-in-isolation fact that becomes contradictory once merged. RDT's own convention is that a supplied default makes a prop effectively optional regardless of what the type annotation says, so `ParsedProp::new` normalizes at the one chokepoint every caller must pass through, rather than trusting every call site across the resolver to remember to do it themselves. The two unit tests directly below the impl (`new_normalizes_required_false_when_default_value_present`, `new_preserves_required_true_when_no_default_value`) pin this exact behavior — treat them as the executable spec for this invariant, not incidental coverage.

`_seal`'s `#[serde(skip)]` means this has zero effect on the wire format — the JSON shape is unchanged; the sealing is purely a compile-time constructor gate, invisible to consumers of `ExtractionOutput` JSON.

## `Diagnostic` / `DiagnosticSeverity` / `DiagnosticCode`

**Source:** `crates/core/src/types/diagnostic.rs`

```rust
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub help: Option<String>,
    pub code: DiagnosticCode,
}

#[non_exhaustive]
pub enum DiagnosticSeverity { Error, Warning, Info }

#[non_exhaustive]
pub enum DiagnosticCode {
    UnresolvableImport, GenericArgumentMismatch, OpaqueType, MaxDepthExceeded,
    Unknown, JsDocDefaultMismatch, ComputedDefault, IndexedAccessOpaque,
    TemplateLiteralOpaque, DiscriminatedUnion, IoError, ExcessiveNesting,
    ParseError, InternalPanic, SkippedCandidate, ComponentKeyCollision,
}
```

Both enums are `#[non_exhaustive]` and derive `Serialize`/`Deserialize` normally — no manual impl here, because neither is recursive. `DiagnosticSeverity` serializes `camelCase`; `DiagnosticCode` serializes `SCREAMING_SNAKE_CASE` (verified by the `internal_panic_code_serializes_as_screaming_snake_case` and `skipped_candidate_code_serializes_screaming_snake_case` tests).

**`DiagnosticCode::Unknown` is intentional dead code, not a gap.** Its doc comment (diagnostic.rs:65-70) states it: reserved headroom for external consumers constructing their own `Diagnostic` outside the extraction pipeline (e.g. a wrapping tool). Nothing in `crates/core` or `crates/cli` is expected to construct it, and it shows up in this crate's own tests only as a placeholder value (`output.rs` tests build `Diagnostic { code: DiagnosticCode::Unknown, .. }` when the specific code doesn't matter to the assertion). Do not "clean this up" by removing it as unused — that's the entire point of it existing.

### Severity → exit code mapping lives on `ExtractionOutput`, not on the enum

**Invariant 5:** `DiagnosticSeverity` has no derived `Ord` — `max_severity()` on `ExtractionOutput` (output.rs:33-35) ranks severities explicitly via a private `severity_rank` function (`Error` = 2, `Warning` = 1, `Info` = 0) rather than relying on enum declaration order. The doc comment is explicit about why: declaration order isn't a promise about severity ranking, and a future contributor reordering the enum's variants (e.g. alphabetizing) would silently invert exit-code behavior if ranking were derived from `Ord`. `exit_code(strict: bool)` (output.rs:42-55) is the single shared mapping reused as-is by `extract`, `watch`, `check --strict`, and `inspect` in the CLI — changing this mapping changes exit-code behavior for every subcommand at once, not just one.

## `ExtractionOutput`, `ComponentEntry`, `ParsedProp` — the top-level shape

**Source:** `crates/core/src/types/output.rs:11-232`

```rust
pub struct ExtractionOutput {
    pub components: BTreeMap<String, ComponentEntry>,
    pub enums: BTreeMap<String, Vec<EnumEntry>>,
    pub diagnostics: Vec<Diagnostic>,
    pub stats: ExtractionStats,
}

pub struct ComponentEntry {
    pub display_name: String,
    pub file_path: Utf8PathBuf,
    pub description: String,
    pub props: BTreeMap<String, ParsedProp>,
    pub inheritance: Vec<InheritedLayer>,
    pub notable_inherited: BTreeMap<String, ParsedProp>,
    pub discriminant_prop: Option<String>,
    pub composes: Vec<String>,
    pub tags: BTreeMap<String, String>,
    pub methods: Vec<()>,
}
```

**Invariant 6:** `components`, `enums`, `props`, and `notable_inherited` are `BTreeMap`, not `FxHashMap` — this is AGENTS.md non-negotiable #2 (`FxHashMap` for internal maps, `BTreeMap` for JSON-output maps) applied at the type level. `ExtractionOutput` and everything nested inside it that's part of the JSON contract is output-facing, so it gets deterministic key order for free from `BTreeMap`'s iteration order — output diffing, snapshot tests, and any consumer that hashes or compares the JSON depend on this being stable across runs, not just correct.

**Invariant 7:** `methods: Vec<()>` is always empty. The doc comment says exactly why: "Always empty for functional components; present for RDT compat." This isn't a placeholder for a future feature — this codebase only extracts functional components, so there's nothing to ever put in this field. Its only purpose is matching the shape react-docgen-typescript consumers already expect (a `methods` array key must exist and be an array, even if empty) so drop-in consumers don't need a schema branch for "the field react-docgen used for class-component methods."

**Invariant 8:** `composes: Vec<String>` holds type names that could not be resolved, explicitly for react-docgen compatibility (doc comment, output.rs:102). This is the "stop-gap" mechanism referenced in recent commit history (`df769ee feat(resolver): surface unresolvable props via react-docgen's own composes field`) — when the resolver can't expand an inherited type into concrete props, rather than silently dropping that inheritance layer's props from the output, it names the type in `composes` so a consumer already handling RDT's `composes` field (a well-known RDT quirk) gets equivalent information instead of silent data loss.

`ComponentEntry::html_element()` (output.rs:113-115) derives the rendered HTML element from `inheritance`, taking the _first_ layer with `html_element: Some(_)` via `find_map`. This means inheritance order matters for this lookup even though it isn't independently validated anywhere in this file — if a component's inheritance chain somehow had two HTML-attribute layers, the outermost one wins by construction, not by any explicit precedence rule.

## `ExtractionStats`

**Source:** `crates/core/src/types/output.rs:785-797`

A flat `Default`-derived struct: `components_extracted`, `components_skipped`, `files_parsed`, `dts_files_parsed`, `dts_cache_hits`, `duration_ms`, `tier1_count`, `tier3_count`, `opaque_count`. No invariants beyond the standard camelCase output convention — flagged here mainly so a future contributor adding a new stat knows the pattern (plain `u32`/`u64` counter, `Default` derive, incremented at the specific pipeline stage it measures) rather than inventing a new shape.

## `GlobalSourceData` and `ResolveState`

**Source:** `crates/core/src/types/global.rs`

`GlobalSourceData` is the merged, cross-file view built once during pipeline orchestration and then read (never mutated) by all parallel resolver workers — `crates/core/AGENTS.md`'s "Resolver" section states it's used via `Arc` in the pipeline specifically so `.clone()` stays cheap. All its maps key by `"${absolute_file_path}:${name}"` (scoped, never bare) — `interfaces`, `type_aliases`, `type_alias_params`, `interface_type_params`, `enums`, `const_arrays` all share this convention, which is what makes `remove_file()` (global.rs:100-111) correct: it can drop every entry belonging to one file with a single `prefix = "{file_path}:"` + `retain(|k, _| !k.starts_with(&prefix))` per map, without needing a reverse index.

**Invariant 9:** `GlobalSourceData::merge()` (global.rs:75-97) performs TypeScript declaration merging for interfaces specifically — if a key already exists (an interface with the same scoped key was already merged, e.g. from a second file augmenting the same module), it _extends_ `props` and `extends` on the existing entry rather than overwriting it (global.rs:78-83). Every other field (`type_aliases`, `enums`, `const_arrays`, ...) uses plain `HashMap::extend`, which _does_ overwrite on key collision. This asymmetry is deliberate — TypeScript interfaces support declaration merging as a language feature; type aliases, enums, and const arrays don't, so a colliding key for those really is either a bug (duplicate scoped key) or an intentional last-write-wins re-parse (e.g. cache invalidation re-merging an updated file after `remove_file()`).

`ResolveState` (global.rs:19-30) is `pub(crate)`, not `pub` — it never crosses into `ExtractionOutput` or any consumer-facing type. It bundles `visited` (cycle detection, `"${file}:${type_name}"` keys), `diagnostics` (accumulated during one `resolve_component()` call), and `in_scope_type_params` (generic placeholder tracking). AGENTS.md's "Resolver" section states explicitly: "`ResolveState` accumulates diagnostics and visited-type tracking for a single resolution call; it is not shared" — each parallel rayon worker gets its own, freshly created per component, never reused or merged across components.

## `react_types.rs` — React/DOM builtin recognition

**Source:** `crates/core/src/react_types.rs`

Pure lookup tables, no file I/O (module doc comment: "compile-time constants derived from `@types/react`"). Three lookups feed the resolver's HTML-attribute-inheritance path:

- `html_element_for(type_name)` — maps a concrete `@types/react` attribute type name (`ButtonHTMLAttributes` → `"button"`) directly. Returns `None` for names that are recognized-but-not-a-single-element (`AriaAttributes`, `SVGAttributes`, `SVGProps`) — that `None` is a meaningful "known, but has no element" result, not "unrecognized."
- `html_element_from_type_arg(type_arg)` — the generic-form counterpart. `SVGAttributes<T>`/`SVGProps<T>`/`HTMLProps<T>` don't bake an element into their own name; the doc comment explains this derives the tag from the caller-supplied `T` (e.g. `HTMLProps<HTMLButtonElement>` → `"button"`) instead of naively stripping/lowercasing the interface name, because that naive transform gives wrong answers for exactly the cases that matter (`HTMLAnchorElement`'s real tag is `a`, not "htmlanchor"). Falls back to `None` — "the prior, safe opaque behavior" — for any type argument not in its explicit table, rather than guessing.
- `is_react_builtin(name, extra)` — terminal-type recognition; `extra` is an `FxHashSet` injection point from `PipelineOptions.extra_builtins`, letting callers extend the builtin list without editing this file.

**Invariant 10:** `parse_react_version(s)` returns `Result<ReactVersion, &str>`, never silently defaulting — the doc comment ties this directly to AGENTS.md non-negotiable #6: a typo like `"react20"` or `"React18"` (wrong case) is an `Err`, not a silent fallback to `REACT_19`. Every caller of this string-to-version mapping (CLI flag, `docgen.config.ts`, NAPI options) shares this one function specifically so the string↔enum mapping can't drift between three separate call sites — a second, slightly-different parser added at one of those call sites would reintroduce exactly the drift risk this function exists to prevent.

`ReactVersion { implicit_children: bool, ref_as_prop: bool }` is a plain data struct (not part of the JSON output contract — it's a pipeline-configuration input, not an extraction result) encoding the React 18/19 behavioral split: React 18's `FC` implicitly includes `children`; React 19's doesn't. React 19 also makes `ref` a plain prop instead of requiring `forwardRef`. `REACT_18`/`REACT_19` are the two supported concrete values; there is no third option — `parse_react_version` is exhaustive over exactly these two strings.

## The msgpack cache's implicit field-order contract (ADR-0003)

**Source:** `docs/adr/0003-positional-msgpack-cache-encoding.md`, `crates/core/src/types/collected.rs` (`SourceData`, `CollectedObjectField`)

Not a `types/output.rs` concern directly, but load-bearing for every type in `collected.rs` that gets cached: the DTS parse cache (`cache.rs`) persists `SourceData` via `rmp_serde::to_vec`/`from_slice` using **positional** (non-named) MessagePack encoding — a struct serializes as an array of field values in declaration order, not a map of field names.

**Invariant 11:** `SourceData`'s field _order_ is part of the wire format, not just its field _set_. A `CACHE_SCHEMA_VERSION` constant in `cache.rs` must be bumped whenever a field is added anywhere but the very end of the struct, removed, or reordered — anything that shifts the decode position of fields relative to what an old cache entry on disk was written with. The ADR calls out `const_arrays` (added mid-struct, not appended) as the concrete precedent for exactly the case this version bump exists to cover.

**Why this is dangerous rather than merely inconvenient:** a forgotten version bump does not fail loudly. `rmp_serde` will happily decode an old, differently-shaped byte sequence into the new struct shape — every field gets _some_ value, just the wrong one, read from whatever position the old encoding happened to put unrelated data. There's no compiler check and no runtime error; it's "plausible-looking but wrong data," which the ADR explicitly calls "the worst kind of bug." Anyone adding a field to `SourceData`, or to any type nested inside it (`CollectedInterface`, `RawProp`, `CollectedTypeAlias`, `ComponentMapping`, ...) that flows into the cached shape, must check whether it lands at the true end of the struct; if not, `CACHE_SCHEMA_VERSION` needs a bump in the same change.

This is also _why_ `CollectedObjectField` (collected.rs:469-475) still plainly derives `Serialize`/`Deserialize` rather than getting folded into `CollectedType`'s manual impl, and why its own test (`collected_object_field_round_trips_through_rmp_serde_positional_encoding`, collected.rs:765-780) exists specifically to exercise `visit_seq` (positional) deserialization rather than `visit_map` — it's the one path that would actually break if someone "simplified" it into a map-based encoding, since `CollectedType`'s own hand-rolled (de)serialization never calls into `CollectedObjectField`'s derive at all (it builds fields manually inside `to_json_value`/`from_json_value`); the derive only matters for whatever _does_ serialize a bare `CollectedObjectField`, which today is exactly this rmp_serde cache path.

## Known gaps (cross-referenced, not re-derived)

- `docs/edge-cases.md` P0-1: `known.rs` shortcut lookup-precedence is inconsistent between `resolver/named.rs` (source-defined types checked first, correct) and `resolver/chain.rs`'s `extends`-clause path (hardcoded shortcuts checked first) — a project defining its own `interface SxProps {...}` and extending it gets silently replaced by the hardcoded MUI opaque shape. This is a resolver call-site bug, not a `types/` shape problem, but it's the kind of thing that would be invisible without knowing `OpaqueDetail::new`'s two call sites (`known.rs` vs. resolver `give_up` sites) both ultimately produce the same `PropType::Opaque` shape — you can't tell from the _type_ alone which path produced a given opaque value.
- `docs/edge-cases.md`: multi-parameter function types degrade to `PropType::Opaque { MultiParamFunction }` via `func.rs:79`'s `give_up` call — this one _does_ push a diagnostic (confirmed reading `func.rs:79` above), which appears to resolve what edge-cases.md describes as inconsistent at `func.rs:54-61`; worth a follow-up check against `docs/edge-cases.md`'s exact line reference if that document predates a fix.
