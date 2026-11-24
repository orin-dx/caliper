# Audit: crates/semantic and its core boundary, for the five recurring review patterns

Read-only. Source: `origin/main` at `f4a88f93` (checked out detached in the agent worktree). Line numbers are for that commit. tsrs sources read: `tsrs_api-0.24.0` (`src/checker/setup.rs`, `src/checker/json.rs`, `src/session.rs`, `src/sourcefiles.rs`), `tsrs_api_codec-0.24.0/src/handle.rs`, `tsrs_checker-0.24.0/src/types.rs`, `tsrs_ast-0.24.0/src/symbolflags.rs`. Nothing was built or run: every finding is from reading, and the ones marked "plausible" were not reproduced.

Classes: (a) default that claims, (b) wire absence conflated, (c) work ahead of a guard, (d) unpinned copied constant, (e) weak test, (f) silent degrade. Severities: **wrong output**, **wasted work**, **weak test**.

## Top 10

1. **The session hook caches an unavailable answer as the component's answer, then stays silent (f, a). Wrong output.** `crates/core/src/pipeline/semantic_hook.rs:302-321` caches every `Settled::Answered`. `read_outcome` (`:416-427`) turns an outcome with no component and no fallback into `Answered { component: None, diagnostics: [] }`. Its warning goes to `once`, which `refine` shows only for the first such answer in the session (`:247-251`, `warned`).
   - Trigger: a Storybook session whose checker times out once on component A (`on_demand.rs:550-555` gives `unavailable("the checker stopped answering: …")` and `forget()`s the snapshot). The checker then recovers. Every later request for A is `Found::Cached`: structural props, no diagnostic, `awaits_checker` false. The host never asks again until a source file changes. The same happens to a component queued behind a tsrs panic: `STOPPED` (`checker.rs:568`) is `Rejected`, which becomes an Info fallback that is cached while the checker restarts.
   - Fix: do not insert into `cache.answers` when `read.once` is non-empty or the fallback reason is transient. Better, have the outcome say whether it is transient (see Model change 2). Attach the unavailable warning to every answer it affects (`affects`), not once per session.
2. **A wire-contract break is an Info-level "kept their structural result", and it is persisted (f, d). Wrong output.** `rpc.rs:1137-1140`: `decode` maps a shape error to `CallError::Rejected("unexpected response shape …")`. `lib.rs:838-840` turns that into a `SemanticFallback`. `semantic_hook.rs:429-440` and the CLI merge report it as `Info` `SemanticBackend`. That is the same code and severity as "not exported by its module", "overloaded call signatures" and the provenance note "resolved by the TypeScript checker". `kept.rs:335` keeps any outcome without diagnostics, and fallbacks are not diagnostics, so the broken answer is replayed from disk on reruns.
   - Trigger: a tsrs bump that renames a required key (`flags`, `id`, `reference`) gives every component a fallback. Nothing fails, and the weekly sees it only as recall drift.
   - Fix: add `CallError::Contract(String)` for decode errors. Treat it as fatal to the run: a `Warning` with its own code, never kept.
3. **Every optional response field is `#[serde(default)]`, so a renamed or moved key decodes as "empty or false" and the converter acts on it (b, a). Wrong output.** `rpc.rs:300-311` and `:385-453`; consumers at `convert.rs:1934`, `:1950`, `:1404`, `:1386`, `instantiate.rs:53` and `declared.rs:70`. Each rename turns into a silent claim:
   - `elementFlags` absent: every tuple element is Required (`unwrap_or(1)`, `convert.rs:1934`).
   - `readonly` absent: tuples become mutable.
   - `texts` absent: a template literal is emitted with no text and all holes dropped (the loop at `:1404` consumes holes only per text).
   - `typeParameters` absent: every generic component is treated as non-generic (`default_arguments` returns `None`).
   - `declarations` absent: no property is "own", React props are no longer `react_declared`, `Module::declares` fails for every export, and `Origin.file` is `""`.
   - `intrinsicName` absent: error types read as a written `any`. `tsrs_real.rs` pins this one.

   `each_parameter_that_chooses_the_answer_changes_it` varies request parameters only. No test varies the response fields we decode. Fix: see Model changes 1 and 4.

4. **`declared_in` hand-parses the node handle, and an unparsable one becomes the path `""`, which reads as "project-own" (a, d). Wrong output.** `convert.rs:374-376` (`splitn(3,'.').nth(2).unwrap_or("")`) and `convert.rs:260-264` (`Origin.file … unwrap_or_default()`).
   - What `""` asserts: not lib, not a dependency, not React. So `is_boundary()` is false (a dependency's interface is expanded instead of named), `react_terminal()` is `None` (`ReactNode` and `CSSProperties` lose their terminals), `is_own` is true, and `is_react_declared` is false.
   - Trigger: tsrs changing the handle format (`"{index}.{kind}.{path}"`, `tsrs_api_codec/src/handle.rs:13`), or a type symbol with no declarations.
   - Fix: call `tsrs_api_codec::parse_node_handle` (public) and return `Option<&str>`. An unknown file means unknown origin: name the type, do not expand it.
5. **Whether tsrs panicked is decided from message text (d, f). Wrong output.** `checker.rs:484` reads `error.kind == ErrorKind::Internal && message.starts_with("panic: ")`, copied from `tsrs_api/src/session.rs:618`. If the wording changes, a session that a panic left half-built keeps answering. Tests use fake sessions that return the string (`checker.rs:793`, `:866`, `lib.rs:998`).
   - Fix: pin it against the real checker with a known trigger. `rpc.rs:102` documents that `getAliasedSymbol` on a plain `default` export panics in the beta.
6. **`getBaseTypes` answering `null` reads as "proven to have no bases" (a, b). Wrong output, plausible.** `rpc.rs:1012-1015` (`bases.unwrap_or_default()`), used by `named_bases` (`convert.rs:1088`, `:1092`). `composes` then treats the type's sources as reached whole and its names list as complete (`known = true`), which allows a collapse.
   - Fix: return `Option<Vec<_>>` and map `None` to `BasesOf::Unknown`, which already exists for exactly this case.
7. **Degrade kinds share one code and one severity, so hosts and harnesses read message text (f, d). Weak detection, wrong decisions.**
   - `awaits_checker` (`semantic_hook.rs:137-142`), the NAPI `awaitsChecker` (`binding/src/lib.rs:471`), compares `message == CHECKER_PENDING`.
   - `gate_harness.rs:35,78` counts checker answers by `message.contains("resolved by the TypeScript checker")`, and `:135` detects the memory bound by `contains("bound (\`max_rss_mib\`)")`.
   - `ondemand_bench.rs:96-98` takes the last diagnostic's message as the reason the checker was unavailable.
   - `scripts/weekly.py` does not read diagnostics at all, so a run past its heap budget, or one with every component on a fallback, looks like a fast run.
   - Unavailable, heap budget, untold files and "stopped" are all `Warning` `SemanticBackend`; pending, provenance and fallback are all `Info` `SemanticBackend`.
   - Fix: see Model change 2.
8. **No TypeScript flag constant is pinned to the locked tsrs (d). Latent wrong output.** `rpc.rs:509-556` holds 28 `TypeFlags`, 3 `SymbolFlags`, 4 `ObjectFlags`, `SignatureFlags.HasRestParameter`, 3 `ElementFlags` and the 4 `TypeFormatFlags` of `PRINT_EXPANDED`. Also unpinned: the literal `1` (`ElementFlags.Required`) at `convert.rs:1934`, `category == 1` (`rpc.rs:594`) and `SignatureKind` 0/1 (`rpc.rs:476-479`).
   - All of them match 0.24.0 today (checked against `tsrs_checker/src/types.rs:172,838,1006,2415,2999` and `tsrs_ast/src/symbolflags.rs`). No test fails on a bump. There is no `Cargo.lock` version test on main; `build.rs:44` only fingerprints the lock for the kept answer.
   - Fix: see Model change 3.
9. **A rejected request while reading a module is reported as "not part of the checker's program" (a, f). Wrong message.** `lib.rs:794-801`: `Ok(None) | Err(CallError::Rejected(_))`.
   - Trigger: a decode error or a nesting refusal (`checker.rs:470`) on `getSymbolOfSourceFile` or `getExportsOfModule`. Those components get a reason that sends users to look at their tsconfig.
   - Fix: give `Rejected` its own reason (`the checker rejected a request: …`) as `:838` does.
10. **A tuple's elements come from `getTypeArguments` without `without_this` (a). Wrong output, plausible.** `convert.rs:1916` and `:1931-1948`. TypeScript's `getTypeArguments` on a tuple reference made by `getTypeWithThisArgument` holds one argument more than the arity: TS slices it in `getElementTypes`.
    - Symptom: an extra element typed `this`, marked Required by the `unwrap_or(1)` default.
    - Fix: take `arguments[..element_flags.len()]`, or apply `without_this` as `type_arguments_of` does. Treat a length mismatch as degraded, not Required.

## (a) Defaults that make a claim

| # | file:line | Default | What it asserts | When the real value differs (input) | Severity | Minimal fix |
| --- | --- | --- | --- | --- | --- | --- |
| a1 | `convert.rs:1934` | `element_flags.get(i).unwrap_or(1)` | the element is required | `elementFlags` renamed or absent; the arguments carry a `this` argument (Top 10); `info` not a tuple target | wrong output | `get(i)` is `None`: degraded tuple |
| a2 | `convert.rs:263` | `Origin.file … unwrap_or_default()` | declared in the project | a symbol without declarations; a changed handle format | wrong output | `Option<String>`; unknown origin is a boundary |
| a3 | `convert.rs:375` | `declared_in … unwrap_or("")` | path is `""` (project) | handle format change | wrong output | `tsrs_api_codec::parse_node_handle` |
| a4 | `rpc.rs:1014` | `bases.unwrap_or_default()` | no base types | `null` answer for an interface whose bases tsrs cannot form | wrong output (plausible) | return `Option`, use `BasesOf::Unknown` |
| a5 | `convert.rs:1950` via `rpc.rs:405` | `readonly: false` | mutable tuple | key renamed; `info` not a tuple target (tsrs writes `readonly` only on targets, `setup.rs:403`) | wrong output | `Option<bool>`; `None` is unknown |
| a6 | `convert.rs:1404` via `rpc.rs:411` | `texts: []` | no text, holes dropped | key renamed (never legitimately empty: TS keeps holes + 1 texts) | wrong output | require `texts.len() == holes + 1`, else degraded |
| a7 | `lib.rs:796` | `Err(Rejected(_))` as `Ok(None)` | file not in the program | decode error, nesting refusal | wrong message | own reason |
| a8 | `component.rs:57-60` | `Module::declares` false on empty `declarations` | export declared in another file | symbol with no declarations | wrong message | say "has no declaration" |
| a9 | `instantiate.rs:71,74,77-79` | missing probe export, `Ok(false)` | a name means something else where instantiated | the probe alias was not declared or not exported | wrong message | separate reason for "probe missing" |
| a10 | `convert.rs:1166-1170` | `invariant`: `Err(_) => false` | the map is not invariant | any `CallError`, `Fatal` and `OverBudget` included, is swallowed | conservative; hides `Fatal` | propagate `Fatal`/`OverBudget`, `Rejected` is false |
| a11 | `semantic_hook.rs:419` | `components.is_empty() && fallbacks.is_empty()` means unavailable | the checker is down for the session | a heap-budget `left` outcome (`heap.rs:297`) or an outcome that omits this key | wrong output (Top 10) | outcome states its status |
| a12 | `convert.rs:1374-1375` | enum literal whose symbol or parent is `None` gives a plain literal | not an enum member | `getSymbolOfType` or `getParentOfSymbol` answering `null` | wrong output, no marker | degraded marker or `Named` from display |
| a13 | `convert.rs:1392` | `display == "true"` else `false` | boolean literal is `false` | any print other than `true`, e.g. a future `intrinsicName` change | wrong output (low) | match `"false"` explicitly, else error |
| a14 | `checker.rs:171` / `on_demand.rs:721` | RSS unreadable (`None`) means `Keep` | under the memory bound | a platform where resident size cannot be read: the bound is off and nothing says so | silent degrade | one Info diagnostic "memory bound not enforced" |
| a15 | `declared.rs:188,192` | `peek(..).unwrap_or_default()` | no description, no tags | a slot left unanswered (unreachable today: the loop re-takes) | smell | `expect`-free `Option` carried to the merge |
| a16 | `rpc.rs:780`, `on_demand.rs:503,659` | `unwrap_or_default()` on values proven `Some` | none | unreachable | smell | `let Some(..) else` with a reason |

Not claims (checked): `declared.rs:293` (`text` is `str_nonempty`, so absent means `""`), `declared.rs:138` (`null` tags mean none), `convert.rs:2050` (guarded by `:2042`), `kept.rs:327` (`false` means re-ask), `kept.rs:167` (stat only; `ReadFile` hashes content).

Core side, plausible: `resolver/relevance_inputs.rs:34`: `mapping.body … unwrap_or_default()` reads "the body reads nothing" when body facts are absent. `relevance::decide` (`relevance/mod.rs:481-482`) then collapses ambient members. Check whether `forwarding` can be `Some` with `mapping.body == None`.

## (b) Response fields we decode: absent for which reasons, and what we conclude

What tsrs 0.24 writes is from `setup.rs:57-88` (signature), `:150-249` (type, index), `:323-365` (symbol) and `:368-465` (type base). `str_nonempty`, `nonzero` and `ids` are omit-if-empty (`json.rs:31-51`).

| Our field (rpc.rs) | tsrs emits | Absent because | We conclude | Two reasons? |
| --- | --- | --- | --- | --- |
| `SymbolWire.declarations` :308 | only if non-empty | empty / renamed | no declaration: not own, not React, not declared in module, `Origin.file` "" | empty vs not carried (rename) |
| `SymbolWire.parent` :310 | only if `Some` | none / renamed | ask each declaration's symbol (extra work only) | harmless |
| `CompactSymbol.file` :374 | only if not transient and not content-mapped (`symbol_owner_file`) | transient **or** content-mapped file | used only as part of memo keys | **two reasons**, no conclusion drawn: safe |
| `TypeInfo.object_flags` :391 | always; adds `MembersResolved` only after a member query (`setup.rs:183-188`) | never | n/a; the value depends on query history. We test only Class/Interface/Reference/Mapped: safe while no code hashes `object_flags` | history-dependent |
| `TypeInfo.value` :393 | always (`null` if not a literal); `±Infinity`/`NaN` as strings | n/a | `as_f64` is `None` for those strings, so the type is named by display (honest) | ok |
| `TypeInfo.intrinsic_name` :399 | only for the Intrinsic branch, non-empty | not intrinsic / boolean literal (Freshable branch first) / renamed | not an error type | rename makes errors `any` (pinned by `tsrs_real.rs`) |
| `TypeInfo.element_flags` :402 | tuple **targets** only, non-empty | empty tuple / not a target / renamed | Required (a1) | **three reasons** |
| `TypeInfo.readonly` :405 | tuple **targets** only | not a target / renamed | mutable (a5) | **two reasons** |
| `TypeInfo.labeled_element_declarations` :408 | only in `Setup::type_response`, only if any label | no labels / built by `type_response_base` / renamed | no labels | two reasons (cosmetic) |
| `TypeInfo.texts` :411 | template literals, non-empty | renamed only | empty template, holes dropped (a6) | rename only, but the drop is silent |
| `TypeInfo.symbol` / `alias_symbol` :417,420 | if `Some` | none / renamed | anonymous type: expanded | rename expands every named type |
| `TypeInfo.alias_type_arguments` :423 | `ids` omitempty | none / renamed | alias has no arguments | rename drops arguments silently |
| `TypeInfo.constraint_type` / `name_type` :427,430 | `nonzero`, only for Mapped | none / renamed | `name_type` absent means "no `as` clause", so maps share counts (`map_key`) | rename makes counts wrong |
| `SignatureInfo.type_parameters` :449 | `ids` omitempty | none / renamed | not generic | rename turns generics off |
| `SignatureInfo.parameters` :452 | only if non-empty | none / renamed | `printed_markers` short-circuit (degrades to opaque later) | safe |
| `SignatureInfo.flags` :446 | always | n/a | — | ok |
| `IndexInfo.is_readonly` :470 | always | n/a | — | ok |
| `BatchAnswer.result` :493 | per answer | method answered `null` / omitted | `null`: `Option` gives `None`, a `Vec` or `bool` fails decode (loud) | two reasons, both read as `null` |
| `JsDocTag.text` :365 | `str_nonempty` | empty | `""` | single reason: ok |
| not decoded | `valueDeclaration`, `checkFlags`, `exportSymbol`, `target`, `fixedLength`, `thisParameter`, `declaration` | — | — | `checkFlags` carries `Readonly` for synthetic properties: a source of readonly without `isReadonlySymbol` |

Summary: 9 fields where absence is turned into a claim (declarations, intrinsicName, elementFlags, readonly, texts, symbol/aliasSymbol, aliasTypeArguments, nameType, typeParameters). In 0.24 most of them have one legitimate reason (empty). The second reason is a key rename or move on a bump, which `#[serde(default)]` hides. `elementFlags` and `readonly` also have a structural second reason: they are written for tuple targets only.

## (c) Work ahead of a guard

| file:line | Work | Guard it precedes | Severity | Fix |
| --- | --- | --- | --- | --- |
| `semantic_hook.rs:224-230` | `std::fs::read` of the whole component file plus a hash, on every `refine` | the answer-cache lookup (`find`) | wasted work (IO per interactive request) | key by the session's revision or generation, or by `(mtime, len)` |
| `convert.rs:1279` | `decode(&text)` of the memoized raw `parts` on every hit | memo hit (`state.parts.get`) | wasted work (re-decodes `Vec<TypeInfo>` per reader) | memoize the decoded `Arc<[TypeInfo]>` |
| `convert.rs:1668-1686` | `symbol_origin`, `origin` and `callable` (up to 3 RPCs, memoized per key) | `enter` (recursion and `MAX_DEPTH` stop) | wasted work on deep recursion | move the boundary check after `cx.encloses` |
| `component.rs:167-168` | `composes()` (base walk) and `as_parents` for every component | `known_prop` memo hits for all props | wasted work on warm memos | skip when `layering` is `None` and every prop is known |
| `convert.rs:1127-1129` | `origin()` RPC to name a cut type | — (after the size check: correct) | ok | — |
| `convert.rs:1991` | readonly batch | `fields_past_size_budget` | fixed on main | — |

The CLI starts early only when no kept answer exists (`cli/src/commands/extract.rs:51-54`), so the "package walk before a cache hit" pattern is not present on main.

## (d) Copied, version-coupled constants

| file:line | Constant | Source in tsrs 0.24 | Pinned by a test that fails on bump? |
| --- | --- | --- | --- |
| `rpc.rs:510-537` | 28 `TypeFlags` | `tsrs_checker/src/types.rs:838` | no |
| `rpc.rs:538-540` | `SymbolFlags` Interface, Alias, Optional | `tsrs_ast/src/symbolflags.rs` | no |
| `rpc.rs:543` | `SignatureFlags::HasRestParameter` | `types.rs:2999` | no |
| `rpc.rs:545-548` | `ObjectFlags` Class, Interface, Reference, Mapped | `types.rs:1006` | no |
| `rpc.rs:551` | `PRINT_EXPANDED` (4 `TypeFormatFlags`) | `types.rs:172` | indirectly (`each_parameter…` asserts printed text) |
| `rpc.rs:553-555`, `convert.rs:1934` | `ElementFlags` (and literal `1`) | `types.rs:2415` | no |
| `rpc.rs:476-479` | `SignatureKind` 0/1 | checker | yes (the `kind` case of `each_parameter…`) |
| `rpc.rs:594` | `DiagnosticCategory.Error == 1` | — | yes (the `files` case asserts an error) |
| `convert.rs:374` | node handle `"{index}.{kind}.{path}"` | `tsrs_api_codec/src/handle.rs:13` (public `parse_node_handle`) | no (tests use literal strings) |
| `checker.rs:484` | `"panic: "` prefix | `tsrs_api/src/session.rs:618` | no (fakes only) |
| `rpc.rs:332-358` | internal names `__@x@id`, `__#id@#x`, well-known symbols | checker escaping | partly (`cases.rs:194-195` real checker) |
| `rpc.rs:1662` `TSRS_READS` | parameter table | `tsrs_api/src/paramfields.rs` (crate-private) | yes: `each_parameter_that_chooses_the_answer_changes_it` |
| `Cargo.lock` tsrs version | 0.24.0 | — | no version test on main (`build.rs` fingerprints only) |

Every value matches 0.24.0 today.

## (e) Weakest 10 tests in crates/semantic

1. `kept.rs:593` `a_run_that_was_refused_a_file_keeps_no_answer`: `assert!(components.is_empty() || !fallbacks.is_empty() || !diagnostics.is_empty())` is nearly vacuous. Assert the exact fallback for the refused file's components.
2. `kept.rs:426,443,490,499,539,553,576,618`: `kept(..).is_some()`, presence only. Only `:410` compares the kept answer to a fresh one. Assert `kept(..) == Some(fresh)`.
3. `kept.rs:434`, `:625`: `format!("{:?}", prop_type).contains("md")` / `"\"c\""` match Debug text. Assert the exact `PropType::Union`.
4. `lib.rs:1116` `static READS` (process-wide), `:1139` `(1..24).contains(&answered)`: timing-dependent range on shared state. Pass the counter in a closure capture, and assert that the answered set equals the unbounded set's prefix.
5. `lib.rs:990-1007`: strips a message prefix and accepts any of three texts; `(1..=2).contains(asked)`. Assert on a status enum, not text.
6. `tests/instantiate.rs:48-51`: `poly.len() >= 260`, `at("onClick").is_some() && at("tone").is_some()`. Assert the types.
7. `tests/exports.rs:181`, `tests/resolve.rs:274`, `tests/watch.rs:130`, `tests/on_demand.rs:129` (parses MiB out of the message): all assert message text. Assert code, severity, `affects`, and a structured field.
8. `rpc.rs:1938` `each_parameter_that_chooses_the_answer_changes_it`: strong on requests. It asserts exact printed text (`"(value?: string | undefined) => void"`), which couples it to the printer, and it varies no response field. Add the response-side twin (Model change 4).
9. `tests/families.rs:246-255`, `tests/collapse_model.rs:482,489,693,695`: `contains` / `.any(..)` on unions and layers, so extra members pass. Use `assert_eq!` on the full list.
10. `tests/relevance_model.rs:140` (`named > 0`), `tests/memo.rs:135` (`!seen.is_empty()`): liveness only.

Also: `checker.rs:793,866`, `lib.rs:998` simulate the tsrs panic with a literal string, so the real wording is never tested. `tests/collapse_model.rs:348` is `#[ignore]` on a tsrs 0.24 gap with no tracking test that fails when tsrs fixes it.

## (f) Fallback and degrade paths

| Path | Where | Diagnostic? (code, severity) | Cached or compared as authoritative? | Can a harness tell by code and severity? |
| --- | --- | --- | --- | --- |
| `Rejected` per component (incl. tsrs panic, `STOPPED`, decode error, nesting refusal) | `lib.rs:838`, `instantiate.rs:141,151,203,206` | fallback only; core reports `Info` `SemanticBackend` | **yes**: kept on disk (`kept.rs:335`), hook cache | no: same as benign fallbacks and provenance |
| `Fatal`, `Timeout`, `Closed`, `Malformed` | `lib.rs:571,600`, `on_demand.rs:554` | `Warning` `SemanticBackend` (`types/semantic.rs:366`) | CLI: not kept (has a diagnostic). Session: **cached** in the hook, shown once (Top 10 #1) | partly: same code as heap, untold, stopped |
| Heap budget | `heap.rs:295-322` | `Warning` `SemanticBackend`, `affects` set | not kept; session path: once-only | partly (shares code; `affects` helps) |
| Memory bound (on-demand) | `on_demand.rs:666-679` | via `stay_down`, then `unavailable` `Warning` | permanent (correct) | no; gate_harness matches text |
| RSS unreadable | `checker.rs:171`, `on_demand.rs:721` | none | — | no |
| Pending (wait elapsed) | `semantic_hook.rs:263-267` | `Info` `SemanticBackend` | not cached (correct) | no; `awaits_checker` compares text |
| Structural fallback reasons (not exported, overloaded, …) | `component.rs:44,119-130` | `Info` | cached (correct: deterministic) | indistinguishable from errors |
| Opaque types (`degraded_type`) | `convert.rs:1231`, `:2026` | per-prop `degraded_by` | cached (correct) | yes (structured) |
| `invariant` error swallowed | `convert.rs:1169` | none | the map is not memoized: safe | n/a |
| tracing-only warnings (panic replaced, recording write) | `checker.rs:577,641,675`, `lib.rs:740` | none (log only) | — | no |

## MODEL AND ARCHITECTURE CHANGES

1. **`Known<T>` at the wire boundary** (`enum Known<T> { Unknown, Settled(T) }`). Every optional response field decodes to it through `#[serde(default = "Known::unknown")]`, and the converter must `match` it. An absent key cannot then be read as `false` or `[]`. Fields that tsrs legitimately omits when empty (`declarations`, `typeParameters`, `aliasTypeArguments`) are `Settled(empty)` only when a sibling proves the key is carried. For example, the field-order list `TYPE_RESPONSE_FIELD_ORDER` lets a test assert presence for a known input (see 4).
   - Cost: about 1 day. Touches `rpc.rs` structs, about 25 read sites in `convert.rs`, `declared.rs`, `component.rs` and `instantiate.rs`.
   - Would have prevented: (1) `settled_readonly` and `valueDeclaration`, and the `unwrap_or(false)` fallback. Also a1, a4, a5 and a6 here.
2. **One degrade channel with stable codes.** Replace `SemanticFallback { reason: String }` and the free-form `SemanticBackend` diagnostics with a `Degraded { kind: DegradeKind, transient: bool, affects, detail }`, where `DegradeKind` is `Pending | Resolved | NotAnswerable(Reason) | Rejected | Contract | Unavailable | HeapBudget | MemoryBound | Untold`. It maps to distinct `DiagnosticCode`s or a `kind` field in JSON.
   - Consequences: `kept::save` and the hook cache refuse `transient || Contract`. `awaits_checker`, gate_harness, ondemand_bench and weekly count by kind. `CallError` gains `Contract`, so decode errors stop being `Rejected`.
   - Cost: 1 to 2 days, plus a changeset, because the JSON diagnostics change.
   - Would have prevented: (5) heap and timeout fallbacks not saying so. Also Top 10 #1, #2, #7 and #9.
3. **A `wire` module that owns every version-coupled constant, with one pin test.** Move `flag::*`, `SignatureKind`, the handle parser (delegate to `tsrs_api_codec::parse_node_handle`), the panic predicate and `TSRS_READS` into `semantic/src/wire.rs`.
   - Pin test (dev-dependency on `tsrs_checker` and `tsrs_ast` at the workspace version): `assert_eq!(flag::STRING, TypeFlags::String.bits() as u64)` for each constant, plus `Cargo.lock`'s `tsrs_api` version == `wire::PINNED` (the PR #519 pattern). A bump then fails in one place that lists what to re-read.
   - Cost: about half a day.
   - Would have prevented: (4) the hard-coded syntax kinds, `TSRS_READS` drifting, and the `file`/`files` mix-up (the pin forces a re-read). Also Top 10 #4, #5 and #8.
4. **Response-side variation test, `varies::<Field>()`.** The twin of `each_parameter_that_chooses_the_answer_changes_it`: for every field a wire struct decodes, a pair of real-checker inputs whose answers differ in that field alone, and a check that it decodes to two different `Settled` values. Exhaustiveness comes from a `const DECODED: &[(&str, &str)]` per struct, generated by a small macro on the struct and checked against the `Deserialize` field list.
   - Cost: about 1 day (fixture plus macro).
   - Would have prevented: (3) the test that varied only `kind`. Also (1) the absences read as values, and Top 10 #3 and #10.
5. **Architecture lint (grep test in `crates/semantic/tests/architecture.rs`).** Fail on `unwrap_or(false)`, `unwrap_or_default()`, `unwrap_or(1)` or `unwrap_or("")` applied to an expression whose receiver comes from `rpc::`/wire types (allow-list by file and line with a reason). Fail on `#[serde(default)]` in `rpc.rs` outside `Known`. Fail on `message ==`/`message.contains(`/`starts_with("panic` outside `wire.rs`. Fail on a `static` in test code.
   - Cost: about 2 hours.
   - Would have prevented: (1), and part of (3) (the process-wide counter) and (5) (text matching).
6. **Guard-first helpers.** `Converter::guarded(info, cx, |..| work)` runs `heap::step`, the memo `recall`, `Flights` and `enter` in one fixed order before any closure that may send a request. The hook keys its cache by revision generation instead of reading the file.
   - Cost: about 1 day; moderate refactor of `object()` and `convert()`.
   - Would have prevented: (2) the classify, decode and package-walk-before-guard findings, and section (c) here.

Order by value per cost: 3, then 5, then 2, then 1, then 4, then 6.

## Not verified

- Nothing was compiled or run. "Plausible" items (Top 10 #6 and #10, the `relevance_inputs.rs:34` note) were not reproduced against tsrs.
- Top 10 #1 was traced through the code (`on_demand.rs:550-555` to `semantic_hook.rs:302-321`). No test was written to show it.
- Whether tsrs 0.24 `getBaseTypes` ever answers `null` for an interface was not checked in `handlers_types.rs`.
- `kept::save` persisting a decode-error fallback was inferred from `kept.rs:335` (checks diagnostics only) and `Reads::settled`. Not run.
