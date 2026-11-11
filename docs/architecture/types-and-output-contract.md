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
    String, Number, Boolean, Null, Undefined, Any, Never, Unknown, Void, BigInt, Symbol, UniqueSymbol,
    StringLiteral(CompactString), NumberLiteral(f64), BoolLiteral(bool), BigIntLiteral(CompactString),
    Union(Vec<CollectedType>), Intersection(Vec<CollectedType>), Array(Box<CollectedType>),
    Tuple(Vec<CollectedTupleElement>), Object(Vec<CollectedObjectField>),
    Callable { signatures: Vec<CollectedSignature>, fields: Vec<CollectedObjectField> },
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
    String, Number, Boolean, Null, Undefined, Any, Never, Unknown, Void, BigInt,
    StringLiteral(String), NumberLiteral(f64), BoolLiteral(bool), BigIntLiteral(BigIntValue),
    UniqueSymbol { name: TypeName }, EnumMember { enum_name: TypeName, member: TypeName, value: EnumMemberValue },
    TemplateLiteral(Vec<TemplatePart>),
    Union(Vec<PropType>), Intersection(Vec<PropType>), Array(Box<PropType>),
    Tuple(Vec<TupleElement>), Readonly(Box<PropType>), Object(Vec<ObjectField>),
    Callable { signatures: Vec<CallSignature>, fields: Vec<ObjectField> },
    Named { name: TypeName, args: Vec<PropType> },
    Recursive { name: TypeName, args: Vec<PropType> },
    Truncated { text: String, limit: TruncationLimit },
    ReactNode, CssProperties,
    Function { type_params: Vec<String>, params: Vec<FunctionParam>, return_type: Box<PropType> }, // FunctionParam: name, type, optional, rest
    Ref { element: Option<String> },
    ElementType,
    HtmlAttributes { element: String, omitted: Vec<String> },
    LiteralUnion { members: Vec<String>, has_default: bool },
    SxProps,
    Opaque(OpaqueDetail),
}
```

Note what disappeared crossing the boundary: `Symbol`, `TypeOf`, `KeyOf`, `AtFile`, `IndexedAccess`, `Conditional`, `Mapped`, `Raw` are all gone from `PropType`. Each either resolves into a concrete `PropType` variant (`Function` becomes `PropType::Function` with its parameters and return type as written, a template literal that does not expand becomes `PropType::TemplateLiteral`) or terminates in `PropType::Opaque` with an `OpaqueReason` explaining exactly which unresolvable TypeScript construct it was (`ConditionalType`, `MappedType`, `IndexedAccess`, ...). This is the resolver's entire job, structurally: eliminate every "can't be sure yet" variant.

## Schema version 5: what the checker states (ADR 0026, slice S1)

`PropType` holds what TypeScript's checker states about a prop's type and the structural path used to lose. Each shape is the checker's own (a rest element is listed by its element type, as `getTypeArguments` lists a tuple's), so the checker converter (slice S2) fills the same variants the structural path does and no projection decides a type. A payload with an invariant is a newtype (`TupleLabel` is an identifier, `BigIntValue` decimal digits after an optional `-`) whose reader rejects anything else; names stay `TypeName` like every other name in the enum. Matches on `PropType` stay exhaustive: the serializer, `operand_string`, `literal_kind`, `visit` and TOON's `format_type_compact` have no wildcard arm.

- **`Tuple(Vec<TupleElement>)`**: each element is `{ element_type, kind, label }`, `kind` one of `Required`, `Optional`, `Rest` (`...T[]`, `element_type` is `T`) and `Variadic` (`...T` of a type that is not a known array or tuple, `element_type` is `T`). The resolver reads a rest element's operand (`resolver/collected.rs`, `tuple_of`): a known tuple is spread in, as TypeScript flattens `[string, ...Tail]`; an array is a `Rest` of its element; anything else is `Variadic`. As TypeScript normalizes a tuple, an optional element before the last required one (`[...[string?], number]`) becomes required and admits `undefined` (`[string | undefined, number]`). Without a label, `?` and `...` bind to a parenthesized composite type (`[(string | number)?]`). Filled by the structural path. JSON: `{ "kind": "tuple", "elements": [{ "type", "label"?, "modifier"?: "optional" | "rest" | "variadic" }] }`.
- **`BigInt`** (schema version 6): the primitive `bigint`, beside `String`, `Number` and `Boolean`. Both paths fill it: the structural path from the `bigint` keyword (`CollectedType::BigInt`, and the keyword in a tuple element's source text) and the checker converter from the `BigInt` type flag. JSON `{ "kind": "bigint" }`; version 5 wrote it as `{ "kind": "named", "name": "bigint" }`, which no declaration can take (TS2457) and a reader could not tell from a type the author named. Every projection prints `bigint`, so the RDT type name and the Storybook control are as they were: Storybook has no control for `bigint`, and a number control would hand a `number` to a prop that needs a `bigint`.
- **`BigIntLiteral(BigIntValue)`**: `10n`, `-1n`. Filled by the structural path. JSON `{ "kind": "bigintLiteral", "value": "-1" }`. A negative number literal (`-1`) is a `NumberLiteral`, read from the prefix expression a literal type allows.
- **`UniqueSymbol { name }`**: `typeof brand` for `declare const brand: unique symbol`. Filled by the structural path through the declared value's alias (`record_declared_value_type`, `resolve_typeof`); a bare `unique symbol` elsewhere, which TypeScript rejects, stays `symbol`. JSON `{ "kind": "uniqueSymbol", "name": "brand" }`.
- **`EnumMember { enum_name, member, value }`**: `Size.Small` when `Size` is an `enum` declaration (not a variant bag, which shares `GlobalSourceData::enums`), with its string or number value. Filled by the structural path for a member reference (`resolver/named.rs`, `enum_member`); a reference to the whole enum stays the union of its literal values. JSON `{ "kind": "enumMember", "enum", "member", "value": <stringLiteral or numberLiteral> }`.
- **`TemplateLiteral(Vec<TemplatePart>)`**: a pattern such as `` `${number}px` ``, its text and `${…}` holes. A template whose holes are finite literal unions is still expanded to its strings, now through `Uppercase`, `Lowercase`, `Capitalize` and `Uncapitalize` too; otherwise the pattern is kept, with each hole as written (names stay names, as a function signature's types do). It is the exact type, so it is not in `degradedBy`; the `TEMPLATE_LITERAL_OPAQUE` info diagnostic still says it lists no values. Filled by the structural path. The `templateLiteral` opaque reason, which only this case wrote, is gone. JSON `{ "kind": "templateLiteral", "parts": [{ "text" } | { "type" }] }`.
- **`Callable { signatures, fields }`**: an object type with call or construct signatures that is not one call signature alone (that stays a `Function`): overloads, `new (…): T`, or signatures beside properties. Each `CallSignature` has a `kind` (`Call`, `Construct`), type parameters, parameters and return type, as written like a function type's. Filled by the structural path for a type literal. JSON `{ "kind": "callable", "signatures": [{ "construct"?: true, "typeParams", "params", "returnType" }], "fields" }`.
- **`Recursive { name, args }`**: a reference back to the type whose expansion encloses it, the nested `Json` in `type Json = string | Json[] | { [key: string]: Json }`, where the cycle check in `resolve_named` used to give `opaque` (`depthExceeded`). It is not a loss and not in `degradedBy`. A reference that is the whole expansion or a direct union or intersection member (`type Loop = Loop`, `type A = A | string`) is a TypeScript error, not a recursive type, and stays `opaque`; only that case reports the `CIRCULAR_REFERENCE` info diagnostic. Filled by the structural path. JSON `{ "kind": "recursive", "name", "args" }`.
- **`Truncated { text, limit }`**: a type cut at a depth or member limit, with `TruncationLimit::Depth` or `Members { total }`; it counts in `degradedBy` as `truncated`, and `holds_opaque` is true for it, so the on-demand checker is asked again. Unread by the structural path, which never truncates by choice (its depth guard is a `depthExceeded` opaque type with a diagnostic): it is for the checker converter's named-boundary policy (S2). A test builds it directly (`types/output/tests.rs`, `streaming_serialize_tests`), and every projection prints it. JSON `{ "kind": "truncated", "text", "limit": { "type": "depth" } | { "type": "members", "total" } }`.

A getter is a property, not a method: `{ get size(): number }` is `{ readonly size: number }`, a getter and a setter of one name are one writable property typed by the getter, and a setter alone is typed by its parameter (`extractor::Accessors`, `method_signature_type`). The tuple shape changed the JSON a version 4 reader reads, so this is one bump for all of them; the parse cache moved to `CACHE_SCHEMA_VERSION` 33.

The projections decide a literal and an option once: `PropType::literal_kind` says what primitive a type is one literal of (an enum member is its value's), and RDT's `enum` shape (`is_literal_union`) and TOON's uncut unions both ask it; `PropType::option_text` is the text RDT writes for an option, an enum member's value (`"small"`) rather than its name. Display text is `raw_string` for all three: TOON prints the new kinds as `raw_string` does, except `truncated(text)` beside `opaque(raw)`.

## LOCKED DESIGN DECISION: manual Serialize/Deserialize on the recursive enums (ADR-0002)

**Source:** `docs/adr/0002-manual-serialize-for-prop-type.md`

`PropType` and `CollectedType` do **not** derive `Serialize`/`Deserialize`. Both implement the serde traits by hand. `CollectedType` (`crates/core/src/types/collected.rs`) round-trips through a private `to_json_value`/`from_json_value` pair that builds and reads a `serde_json::Value`. `PropType` (`crates/core/src/types/output.rs`) streams its `Serialize` impl straight into the serializer as a map with `kind` first and the fields in a fixed order (an `OpaqueReason` is written by the private `WireReason` as `{ type, ... }`), and deserializes through a private `from_tagged_value` over a `serde_json::Value`.

**Why:** these are recursive enums (`Union(Vec<Self>)`, `Object(Vec<Field>)` where `Field` contains `Self`, `Array(Box<Self>)`, ...). A derived, tagged (`#[serde(tag = "kind")]`) implementation on a recursive enum needed `#![recursion_limit = "2048"]` to compile — each nesting level wraps the generated (de)serializer in another codegen layer, and a real component prop tree blows past the default limit (128). Manual impls sidestep the codegen recursion entirely; they're just recursive Rust functions, which the compiler doesn't limit the same way.

**Do not "simplify" this back to `#[derive(Serialize, Deserialize)]`.** It looks removable — it's a lot of boilerplate for something serde could theoretically generate — but doing so reintroduces the recursion-limit dependency, and worse, silently drops the manual NaN/Infinity handling described below.

**Correction to the ADR itself, verified against current code:** ADR-0002 states "`PropType`, `CollectedType`, and `OpaqueReason` don't derive Serialize/Deserialize." This is stale for `OpaqueReason`. The current source (`OpaqueReason` in `types/output.rs`) shows:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpaqueReason {
    ConditionalType, MappedType, ModuleAugmentation,
    RuntimeDependent { function_name: String },
    UnresolvableImport { specifier: String },
    PandaCodegenMissing, DepthExceeded,
    IndexedAccess { expression: String },
    UnsupportedExpression,
}
```

`OpaqueReason` derives normally. It isn't recursive — every variant's payload is a `String` or a unit — so it never hit the recursion-limit problem the ADR is about. The _serialization of an `OpaqueReason` value_ is still hand-assembled by `WireReason` (`types/output.rs`, matching each reason to its own `{ type, ... }` shape) when `PropType`'s `Serialize` writes the `Opaque` variant, and hand-parsed in `PropType::from_tagged_value` — but that's because it's embedded inside `PropType`'s own manual impl, not because `OpaqueReason` itself needs one. This is a small, real documentation bug in ADR-0002 worth a one-line fix, independent of anything else in this file.

Also worth noting as **resolved, not a gap:** the ADR flagged `#![recursion_limit = "2048"]` in `lib.rs` as "left over from the old derive-based design... worth deleting as a follow-up." As of the current source, `crates/core/src/lib.rs` has no `recursion_limit` attribute at all — that follow-up has already happened. The ADR's "Consequences" section is stale on this point too.

### The "no wildcard matches" convention

**Invariant 1:** Every match on `PropType` or `CollectedType` inside the manual serde impls, and by extension idiomatically everywhere else in the resolver, must be exhaustive — no `_ => ...` fallthrough arm.

**Why it's load-bearing:** a derived serde impl gives you a compiler error the moment you add a new enum variant and forget to handle it in serialization. A hand-written match loses that safety net _unless every match stays exhaustive_ — a wildcard arm would silently absorb a new variant into whatever the wildcard does (often `Opaque` or a panic), and nothing would tell you serialization is now lossy for that variant. `PropType`'s `Serialize` impl and `from_tagged_value` (`types/output.rs`), and `to_json_value`/`from_json_value` in `collected.rs`, are all written this way. If you add a `PropType` variant, the compiler _will_ fail the exhaustiveness check in the `Serialize` impl and force you to handle it — but only because nobody put a wildcard there. Adding one defeats the entire safety mechanism this ADR relies on.

### NaN/Infinity as a wire-format special case

**Invariant 2:** `PropType::NumberLiteral(f64)` and `CollectedType::NumberLiteral(f64)` encode non-finite values (`NaN`, `Infinity`, `-Infinity`) as JSON strings (`"NaN"`, `"Infinity"`, `"-Infinity"`), not as JSON numbers, and the read side must check for a string before falling back to `as_f64()`.

**Why:** `serde_json::Number` (and JSON itself) cannot represent non-finite floats — `serde_json::json!(f64::NAN)` silently becomes `null`. Without the string-tag workaround, a `NumberLiteral(NaN)` would round-trip as `0.0` (the comments in `PropType`'s `Serialize` impl and in `CollectedType::to_json_value`) or, in `CollectedType`'s case, fail every match arm in `from_json_value` and fall through to `Raw`. Both files have explicit round-trip tests for this (`number_literal_roundtrip_tests` in `output.rs`, the NaN/Infinity/-Infinity tests in `collected.rs`'s `mod tests`) — treat those as regression guards, not optional coverage.

## LOCKED DESIGN DECISION: `OpaqueDetail::new` vs `OpaqueDetail::give_up`

**Source:** `crates/core/src/types/output.rs` (`OpaqueDetail`), cross-referenced against call sites in `crates/core/src/known.rs` and `crates/core/src/resolver/*.rs`

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

- `resolver/collected.rs`, `resolver/primitives.rs`, `resolver/template.rs`, `resolver/named.rs` — every one of these resolver-internal degradation sites calls `OpaqueDetail::give_up`, each with its own specific `OpaqueReason` (`DepthExceeded`, `MappedType`, `ConditionalType`, `UnsupportedExpression`, `IndexedAccess`) and a diagnostic message tailored to that reason.
- `known.rs` — every call is `OpaqueDetail::new`, for the documented exception: `known.rs` has no `ResolveState` of its own to push a diagnostic onto (it's a lookup table, not a resolution pass), so it pushes its diagnostic separately through `push_known_opaque_diagnostic` (`known.rs`) at the call site that invokes the lookup, rather than inline in the constructor.
- `output.rs`'s own `from_tagged_value` — deserialization uses `OpaqueDetail::new` for both the "opaque" JSON tag and the catch-all unknown-`kind` fallback (line 712). This is deserialization, not resolution — there is no `ResolveState` in scope during deserialization, and re-diagnosing a value that's simply being read back off disk/wire would be meaningless (the diagnostic, if any, was already emitted and persisted when the value was first produced).

Read `resolver-type-resolution.md` for the call-site-level reasoning about _why_ each of those resolver locations gives up; this file's job is only the type-level contract: `Opaque` values arising from live resolution are diagnostic-carrying by construction, and the type system (via field privacy + two narrowly-scoped constructors) is what makes that true rather than a convention someone has to remember.

## LOCKED DESIGN DECISION: `ParsedProp`'s sealed-field constructor

**Source:** `crates/core/src/types/output.rs` (`ParsedProp`)

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

**Invariant 4:** a `ParsedProp` can never exist with `required: true` and `default_value: Some(_)` simultaneously. `ParsedProp::new` (`types/output.rs`) enforces this by construction:

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
    pub affects: Vec<String>, // omitted from JSON when empty
}

#[non_exhaustive]
pub enum DiagnosticSeverity { Error, Warning, Info }

#[non_exhaustive]
pub enum DiagnosticCode {
    UnresolvableImport, GenericArgumentMismatch, OpaqueType, MaxDepthExceeded,
    Unknown, JsDocDefaultMismatch, IndexedAccessOpaque,
    TemplateLiteralOpaque, DiscriminatedUnion, IoError, ExcessiveNesting,
    ParseError, InternalPanic, SkippedCandidate, ComponentKeyCollision, DuplicateEnumName, CircularReference, MissingTypeLibrary,
}
```

Both enums are `#[non_exhaustive]` and derive `Serialize`/`Deserialize` normally — no manual impl here, because neither is recursive. `DiagnosticSeverity` serializes `camelCase`; `DiagnosticCode` serializes `SCREAMING_SNAKE_CASE` (verified by the `internal_panic_code_serializes_as_screaming_snake_case` and `skipped_candidate_code_serializes_screaming_snake_case` tests).

**A diagnostic's identity is all of its fields except `affects`.** `Revision::assemble` merges identical diagnostics into one and lists, in `affects`, the output keys of every component whose resolution reported it (sorted). File- and run-level diagnostics (parse errors, missing source directories) have no `affects`. The output's `diagnostics` are sorted by file, line, column and message, then severity by explicit rank, so a watch snapshot and a cold extract list them identically. Before this, one conditional type in Mantine was reported 13,358 times.

**`DiagnosticCode::Unknown` is intentional dead code, not a gap.** Its doc comment (`DiagnosticCode::Unknown` in `types/diagnostic.rs`) states it: reserved headroom for external consumers constructing their own `Diagnostic` outside the extraction pipeline (e.g. a wrapping tool). Nothing in `crates/core` or `crates/cli` is expected to construct it, and it shows up in this crate's own tests only as a placeholder value (`output.rs` tests build `Diagnostic { code: DiagnosticCode::Unknown, .. }` when the specific code doesn't matter to the assertion). Do not "clean this up" by removing it as unused — that's the entire point of it existing.

### Severity → exit code mapping lives on `ExtractionOutput`, not on the enum

**Invariant 5:** `DiagnosticSeverity` has no derived `Ord` — `ExtractionOutput::max_severity` (`types/output.rs`) ranks severities explicitly via a private `severity_rank` function (`Error` = 2, `Warning` = 1, `Info` = 0) rather than relying on enum declaration order. The doc comment is explicit about why: declaration order isn't a promise about severity ranking, and a future contributor reordering the enum's variants (e.g. alphabetizing) would silently invert exit-code behavior if ranking were derived from `Ord`. `exit_code(strict: bool)` (`types/output.rs`) is the single shared mapping reused as-is by `extract`, `watch`, `check --strict`, and `inspect` in the CLI — changing this mapping changes exit-code behavior for every subcommand at once, not just one.

## `ExtractionOutput`, `ComponentEntry`, `ParsedProp` — the top-level shape

**Source:** `crates/core/src/types/output.rs` (`ExtractionOutput`, `ComponentEntry`)

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
    pub detected_by: DetectionRule,
    pub exported: bool,
    pub exports: Vec<ComponentExport>,
}
```

The Rust types hold props inline, but the canonical JSON does not (ADR 0009, `types/output/interned.rs`). `ExtractionOutput` serializes each distinct `ParsedProp` once, in a top-level `props` table keyed by id, and each component's `props` and `notableInherited` map a prop name to an id. An id is the FNV-1a 64 hash of the prop's compact JSON as 16 hex digits, so identical props share one entry and keep their id across watch revisions. `IncrementalUpdate` writes its `updatedComponents` the same way, with a `props` table holding every prop they refer to. Since `schemaVersion` 2 (ADR 0018) a component's `props` is an array of prop set ids, one per declaring type, into a top-level `propSets` table of prop name to prop id; an inheritance layer's `omitted` and a diagnostic's `affects` are ids into a `nameLists` table. A `ComponentEntry` serialized on its own (snapshot tests, `ExtractionOutput`'s `Deserialize` after expanding ids) still holds its props inline.

**Invariant 6a:** two different props never share an id. `PropTable::add` compares the JSON of every prop added under an existing id and fails serialization on a mismatch, rather than letting one prop stand in for another.

`detected_by` (`detectedBy` in JSON) is the extractor rule that recognized the component, so a consumer can tell an explicit declaration form (`functionDeclaration`, `forwardRef`, `fcAnnotation`, `classComponent`, ...) from an inference (`typedFunctionExpression`, `wrapperTypeArgument`). It is the grouping the per-rule precision sample uses (`docs/precision.md`).

`degraded_by` (`degradedBy` in JSON) on a `ParsedProp` lists why part of its type is `Opaque`: the reason kinds of the opaque members (`conditionalType`, `mappedType`, `indexedAccess`, ...), each once, in type order, an index signature's key (``[k: `data-${string}`]: T``) included. `ParsedProp::new` derives it from `prop_type` (`PropType::degraded_by`), so it never disagrees with the type, and a prop the resolver evaluated in full has no field (`skip_serializing_if`), so a reader of earlier output ignores it and `SCHEMA_VERSION` stays (ADR 0017). A name left as an unexpanded `Named` reference is not `Opaque` and is not listed: its warning is in `diagnostics`, which name the component and not the prop. TOON writes it as a `degradedBy` column only when some prop has one (`toon.rs`). TOON also writes a prop's `tags` entries `deprecated`, `see` and `since` as a `tags` column (`deprecated=use X;since=2.1`) only when some prop has one, a function type's parameter names (`(value: string) => void`), every member of a literal union and of a union with a literal member, and a component's `discriminantProp` as a `discriminant:` line. It cuts a union of types past 8 members and an intersection past 4 with `...(+N)`; the canonical JSON holds all of them. A discriminated union whose members are inline object literals has no `discriminantProp` in the canonical JSON (the resolver finds one only among named members), so TOON cannot name it.

`exported` says whether the component's module exports it: an exported declaration, an `export { X as Y }` or `export default X` of a local, or a sub-component attached to an exported component (`Parent.Sub = Sub`, `Object.assign(Parent, { Sub })`). Canonical output keeps unexported components; `--format rdt` and `--format storybook` omit them unless `--include-unexported` is passed, as react-docgen-typescript only documents exports.

`exports` (schema version 7, ADR 0027) is how the module exports it: one `{ name, members }` per way, where `name` is the export name (`default` for the default export, `Card` for `export { _Card as Card }`) and `members` the properties an attached sub-component is reached through (`Menu.Item = Item` beside `export default Menu` is `default` with members `["Item"]`). A reader pairs a component by file and export, never by `displayName`: `_Card` and `Card` can be two components of one file, and `Item` can be a component's name and the name of another's attachment. A re-export (`export { Card } from './Card'`) is in no component's `exports`: the component belongs to the file that declares it. `exported` is `!exports.is_empty()`.

**Invariant 6:** `components`, `enums`, `props`, and `notable_inherited` are `BTreeMap`, not `FxHashMap` — this is AGENTS.md non-negotiable #2 (`FxHashMap` for internal maps, `BTreeMap` for JSON-output maps) applied at the type level. `ExtractionOutput` and everything nested inside it that's part of the JSON contract is output-facing, so it gets deterministic key order for free from `BTreeMap`'s iteration order — output diffing, snapshot tests, and any consumer that hashes or compares the JSON depend on this being stable across runs, not just correct.

**Invariant 7:** `methods: Vec<()>` is always empty. The doc comment says exactly why: "Always empty for functional components; present for RDT compat." This isn't a placeholder for a future feature — this codebase only extracts functional components, so there's nothing to ever put in this field. Its only purpose is matching the shape react-docgen-typescript consumers already expect (a `methods` array key must exist and be an array, even if empty) so drop-in consumers don't need a schema branch for "the field react-docgen used for class-component methods."

**Invariant 8:** `composes: Vec<String>` holds type names that could not be resolved, explicitly for react-docgen compatibility (doc comment on `ComponentEntry::composes`). This is the "stop-gap" mechanism referenced in recent commit history (`df769ee feat(resolver): surface unresolvable props via react-docgen's own composes field`) — when the resolver can't expand an inherited type into concrete props, rather than silently dropping that inheritance layer's props from the output, it names the type in `composes` so a consumer already handling RDT's `composes` field (a well-known RDT quirk) gets equivalent information instead of silent data loss.

`ComponentEntry::html_element()` (`types/output.rs`) derives the rendered HTML element from `inheritance`, taking the _first_ layer with `html_element: Some(_)` via `find_map`. This means inheritance order matters for this lookup even though it isn't independently validated anywhere in this file — if a component's inheritance chain somehow had two HTML-attribute layers, the outermost one wins by construction, not by any explicit precedence rule.

## `ExtractionStats`

**Source:** `crates/core/src/types/output.rs` (the extraction stats struct)

A flat `Default`-derived struct: `components_extracted`, `components_skipped`, `files_parsed`, `dts_files_parsed`, `dts_cache_hits`, `duration_ms`, `tier1_count`, `tier3_count`, `opaque_count`. No invariants beyond the standard camelCase output convention — flagged here mainly so a future contributor adding a new stat knows the pattern (plain `u32`/`u64` counter, `Default` derive, incremented at the specific pipeline stage it measures) rather than inventing a new shape.

## `GlobalSourceData` and `ResolveState`

**Source:** `crates/core/src/types/global.rs`

`GlobalSourceData` is the merged, cross-file view built once during pipeline orchestration and then read (never mutated) by all parallel resolver workers — `crates/core/AGENTS.md`'s "Resolver" section states it's used via `Arc` in the pipeline specifically so `.clone()` stays cheap. All its maps key by `"${absolute_file_path}:${name}"` (scoped, never bare) — `interfaces`, `type_aliases`, `type_alias_params`, `interface_type_params`, `enums`, `const_arrays` all share this convention, which is what makes `GlobalSourceData::remove_file` (`types/global.rs`) correct: it can drop every entry belonging to one file with a single `prefix = "{file_path}:"` + `retain(|k, _| !k.starts_with(&prefix))` per map, without needing a reverse index.

**Invariant 9:** `GlobalSourceData::merge()` (`types/global.rs`) performs TypeScript declaration merging for interfaces specifically — if a key already exists (an interface with the same scoped key was already merged, e.g. from a second file augmenting the same module), it _extends_ `props` and `extends` on the existing entry rather than overwriting it. Every other field (`type_aliases`, `enums`, `const_arrays`, ...) uses plain `HashMap::extend`, which _does_ overwrite on key collision. This asymmetry is deliberate — TypeScript interfaces support declaration merging as a language feature; type aliases, enums, and const arrays don't, so a colliding key for those really is either a bug (duplicate scoped key) or an intentional last-write-wins re-parse (e.g. cache invalidation re-merging an updated file after `remove_file()`).

`ResolveState` (`types/global.rs`) is `pub(crate)`, not `pub` — it never crosses into `ExtractionOutput` or any consumer-facing type. It bundles `visited` (cycle detection, `"${file}:${type_name}"` keys), `diagnostics` (accumulated during one `resolve_component()` call), and `scope` (a `TypeScope`: the generic type parameters in scope, tracked as placeholders). AGENTS.md's "Resolver" section states explicitly: "`ResolveState` accumulates diagnostics and visited-type tracking for a single resolution call; it is not shared" — each parallel rayon worker gets its own, freshly created per component, never reused or merged across components.

## `react_types.rs` — React/DOM builtin recognition

**Source:** `crates/core/src/react_types.rs`

Pure lookup tables, no file I/O (module doc comment: "compile-time constants derived from `@types/react`"). Three lookups feed the resolver's HTML-attribute-inheritance path:

- `html_element_for(type_name)` — maps a concrete `@types/react` attribute type name (`ButtonHTMLAttributes` → `"button"`) directly. Returns `None` for names that are recognized-but-not-a-single-element (`AriaAttributes`, `SVGAttributes`, `SVGProps`) — that `None` is a meaningful "known, but has no element" result, not "unrecognized."
- `html_element_from_type_arg(type_arg)` — the generic-form counterpart. `SVGAttributes<T>`/`SVGProps<T>`/`HTMLProps<T>` don't bake an element into their own name; the doc comment explains this derives the tag from the caller-supplied `T` (e.g. `HTMLProps<HTMLButtonElement>` → `"button"`) instead of naively stripping/lowercasing the interface name, because that naive transform gives wrong answers for exactly the cases that matter (`HTMLAnchorElement`'s real tag is `a`, not "htmlanchor"). Falls back to `None` — "the prior, safe opaque behavior" — for any type argument not in its explicit table, rather than guessing.
- `is_react_builtin(name, extra)` — terminal-type recognition; `extra` is an `FxHashSet` injection point from `PipelineOptions.extra_builtins`, letting callers extend the builtin list without editing this file.

**Invariant 10:** Nothing in this module or in `PipelineOptions` selects a React major version. Extraction does not branch on whether the project uses React 18 or 19: `ref` is extracted like any other member the props type contains (see `docs/rdt-coverage.md`), and an implicit `children` comes only from the types the source names. A `reactVersion` key in `docgen.config.ts` is rejected by name (`DocgenConfigSchema` uses `deny_unknown_fields`), so a config cannot appear to select a version it has no effect on. Do not reintroduce a version switch without a code path that reads it and a test that shows the output differing.

## The msgpack cache's implicit field-order contract (ADR-0003)

**Source:** `docs/adr/0003-positional-msgpack-cache-encoding.md`, `crates/core/src/types/collected.rs` (`SourceData`, `CollectedObjectField`)

Not a `types/output.rs` concern directly, but load-bearing for every type in `collected.rs` that gets cached: the DTS parse cache (`cache.rs`) persists `SourceData` via `rmp_serde::to_vec`/`from_slice` using **positional** (non-named) MessagePack encoding — a struct serializes as an array of field values in declaration order, not a map of field names.

**Invariant 11:** `SourceData`'s field _order_ is part of the wire format, not just its field _set_. A `CACHE_SCHEMA_VERSION` constant in `cache.rs` must be bumped whenever a field is added anywhere but the very end of the struct, removed, or reordered — anything that shifts the decode position of fields relative to what an old cache entry on disk was written with. The ADR calls out `const_arrays` (added mid-struct, not appended) as the concrete precedent for exactly the case this version bump exists to cover.

**Why this is dangerous rather than merely inconvenient:** a forgotten version bump does not fail loudly. `rmp_serde` will happily decode an old, differently-shaped byte sequence into the new struct shape — every field gets _some_ value, just the wrong one, read from whatever position the old encoding happened to put unrelated data. There's no compiler check and no runtime error; it's "plausible-looking but wrong data," which the ADR explicitly calls "the worst kind of bug." Anyone adding a field to `SourceData`, or to any type nested inside it (`CollectedInterface`, `RawProp`, `CollectedTypeAlias`, `ComponentMapping`, ...) that flows into the cached shape, must check whether it lands at the true end of the struct; if not, `CACHE_SCHEMA_VERSION` needs a bump in the same change. Content changes with the same shape need no bump: the payload header's source fingerprint discards the cache whenever the crate's code changes.

This is also _why_ `CollectedObjectField` (`types/collected.rs`) still plainly derives `Serialize`/`Deserialize` rather than getting folded into `CollectedType`'s manual impl, and why its own test (`collected_object_field_round_trips_through_rmp_serde_positional_encoding`, in `types/collected.rs`) exists specifically to exercise `visit_seq` (positional) deserialization rather than `visit_map` — it's the one path that would actually break if someone "simplified" it into a map-based encoding, since `CollectedType`'s own hand-rolled (de)serialization never calls into `CollectedObjectField`'s derive at all (it builds fields manually inside `to_json_value`/`from_json_value`); the derive only matters for whatever _does_ serialize a bare `CollectedObjectField`, which today is exactly this rmp_serde cache path.

## Known gaps (cross-referenced, not re-derived)

- `known.rs` shortcut lookup precedence once differed between `resolver/named.rs` (source-defined types first) and `resolver/chain.rs`'s `extends`-clause path (hardcoded shortcuts first), so a project's own `interface SxProps {...}` was silently replaced by the hardcoded MUI opaque shape; both now share `resolver/precedence.rs` (see `resolver-type-resolution.md`). It was a resolver call-site bug, not a `types/` shape problem, but it's the kind of thing that would be invisible without knowing `OpaqueDetail::new`'s two call sites (`known.rs` vs. resolver `give_up` sites) both ultimately produce the same `PropType::Opaque` shape — you can't tell from the _type_ alone which path produced a given opaque value.
