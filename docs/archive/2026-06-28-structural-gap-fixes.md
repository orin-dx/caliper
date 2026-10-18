# Structural Gap Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 5 structural resolver gaps discovered during the RDT compatibility audit — all are pure AST analysis, no type checker required.

**Architecture:** Each gap has a precisely identified root cause in a single file. The fixes are independent: method-sig params (extractor), React namespace (react_types + named.rs), Readonly (extractor/alias.rs), inline Pick/Omit (resolver/chain.rs). Snapshots are regenerated once at the end.

**Tech Stack:** Rust, OXC 0.135 AST types, `cargo test -p oxc-react-docgen-core`, `INSTA_UPDATE=always` for snapshot regeneration

---

## Background

Five gaps found in `docs/rdt-coverage.md` and `docs/type-checker-integration.md`. These are NOT the deferred gaps (generics, conditionals, mapped types) — those require the Corsa API (TypeScript 7.1, est. Q1 2027). These five are fixable today.

Run tests with: `cargo test -p oxc-react-docgen-core` Snapshot test specifically: `cargo test -p oxc-react-docgen-core --test snapshots` Regenerate snapshots: `INSTA_UPDATE=always cargo test -p oxc-react-docgen-core --test snapshots`

Non-negotiables from CLAUDE.md:

- No `unwrap()` outside `#[cfg(test)]` — use `?`
- `FxHashMap` for internal maps; `BTreeMap` for JSON-output maps
- `CompactString` for type/prop names in hot paths
- No AST refs escape `parse_file()` — allocator is local per call
- Always emit `Diagnostic` when degrading — never fail silently

---

## File Map

| File | What changes |
| --- | --- |
| `crates/core/src/extractor/mod.rs` | Fix TSMethodSignature at lines 359-374 (object field) and 476-492 (RawProp) |
| `crates/core/src/react_types.rs` | Add SVGAttributes, SVGProps, HTMLProps, ComponentRef, JSXElementConstructor to `html_element_for` + `is_react_builtin` |
| `crates/core/src/resolver/named.rs` | Add SVGAttributes, SVGProps, HTMLProps, ComponentRef, JSXElementConstructor to step-6 silent list |
| `crates/core/src/extractor/alias.rs` | Add `"Readonly"` arm before `_ =>` wildcard |
| `crates/core/src/resolver/chain.rs` | Add step 0.5 before existing step 1 to intercept `Pick<T,K>` / `Omit<T,K>` with non-empty type_args |
| `fixtures/rdt-compat/svg-icon.tsx` | New fixture exercising SVGAttributes + HTMLProps |
| `crates/core/tests/snapshots/snapshots__snapshot_rdt_compat.snap` | Regenerated — controlled.tsx gains `eventType:"string"`, pick-source.tsx gains disabled/type/form props |
| `docs/rdt-coverage.md` | Mark gaps as fixed, update bug status |
| `docs/type-checker-integration.md` | Already written (not regenerated here) |

---

## Task 1: Fix TSMethodSignature parameter extraction

**Goal:** `onValueChange?(value: string): void` should emit `eventType: "string"` instead of `eventType: "..."`.

**Root cause:** Two sites in `extractor/mod.rs` hardcode params as `vec![CollectedType::Raw("...".into())]` instead of reading `ms.params.items`. The correct pattern is the TSFunctionType arm at lines 220-236, which maps over `f.params.items`.

The difference between TSFunctionType and TSMethodSignature return type:

- `TSFunctionType`: `f.return_type` is `Box<TSTypeAnnotation>` (not optional) → `self.ts_type_to_collected(&f.return_type.type_annotation)`
- `TSMethodSignature`: `ms.return_type` is `Option<Box<TSTypeAnnotation>>` → `.as_ref().map(...).unwrap_or(CollectedType::Void)`

**Files:**

- Modify: `crates/core/src/extractor/mod.rs:359-374` (ts_signature_to_object_field)
- Modify: `crates/core/src/extractor/mod.rs:476-492` (collect_property_signature)
- Test: `cargo test -p oxc-react-docgen-core --test snapshots`

- [ ] **Step 1: Fix site 1 — `ts_signature_to_object_field` (lines 359-374)**

Replace the TSMethodSignature arm in `ts_signature_to_object_field`:

```rust
TSSignature::TSMethodSignature(sig) => {
    let name = match &sig.key {
        PropertyKey::StaticIdentifier(id) => id.name.as_str().to_owned(),
        PropertyKey::StringLiteral(s) => s.value.as_str().to_owned(),
        _ => return None,
    };
    let params: Vec<CollectedType> = sig
        .params
        .items
        .iter()
        .map(|p| {
            p.type_annotation
                .as_ref()
                .map(|ta| self.ts_type_to_collected(&ta.type_annotation))
                .unwrap_or(CollectedType::Any)
        })
        .collect();
    let return_type = sig
        .return_type
        .as_ref()
        .map(|rt| self.ts_type_to_collected(&rt.type_annotation))
        .unwrap_or(CollectedType::Void);
    Some(CollectedObjectField {
        name,
        collected_type: CollectedType::Function {
            params,
            return_type: Box::new(return_type),
        },
        required: !sig.optional,
        description: String::new(),
    })
}
```

- [ ] **Step 2: Fix site 2 — `collect_property_signature` (lines 476-492)**

Replace the TSMethodSignature arm in `collect_property_signature`:

```rust
TSSignature::TSMethodSignature(ms) => {
    let name = ms.key.static_name()?.to_string();
    let description = self.find_jsdoc(ms.span.start);
    let tags = self.extract_jsdoc_tags(ms.span.start);
    let params: Vec<CollectedType> = ms
        .params
        .items
        .iter()
        .map(|p| {
            p.type_annotation
                .as_ref()
                .map(|ta| self.ts_type_to_collected(&ta.type_annotation))
                .unwrap_or(CollectedType::Any)
        })
        .collect();
    let return_type = ms
        .return_type
        .as_ref()
        .map(|rt| self.ts_type_to_collected(&rt.type_annotation))
        .unwrap_or(CollectedType::Void);
    Some(RawProp {
        name,
        collected_type: CollectedType::Function {
            params,
            return_type: Box::new(return_type),
        },
        required: !ms.optional,
        description,
        tags,
        span_start: ms.span.start,
        span_end: ms.span.end,
    })
}
```

- [ ] **Step 3: Verify compilation**

```bash
cargo build -p oxc-react-docgen-core 2>&1 | head -40
```

Expected: no errors. If OXC's `FormalParameter` field for method params is named differently than `params`, check with:

```bash
grep -n "TSMethodSignature\|FormalParameter" crates/core/src/extractor/mod.rs | head -20
```

- [ ] **Step 4: Run tests (snapshots will change — don't update yet)**

```bash
cargo test -p oxc-react-docgen-core 2>&1 | tail -20
```

Expected: snapshot tests fail because `onValueChange` now shows `eventType:"string"` not `"..."`. Unit tests should all pass. The failure is expected — snapshot update is Task 5.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/extractor/mod.rs
git commit -m "fix: extract TSMethodSignature params instead of hardcoding ellipsis"
```

---

## Task 2: Add React namespace recognitions

**Goal:** `SVGAttributes`, `SVGProps`, `HTMLProps`, `ComponentRef`, `JSXElementConstructor` should not produce UnresolvableImport warnings. `SVGAttributes` and `HTMLProps` should map to HTML elements for `notableInherited`.

**Root cause (three sites):**

1. `react_types.rs:html_element_for()` — maps `HTMLAttributes<T>` → element string. Missing: `SVGAttributes`, `HTMLProps`
2. `react_types.rs:is_react_builtin()` — terminal React types. Missing: `ComponentRef`, `JSXElementConstructor`, `SVGAttributes`, `SVGProps`, `HTMLProps`
3. `resolver/named.rs` step-6 silent list — suppresses UnresolvableImport for known harmless types. Missing: same 5 names.

**Files:**

- Modify: `crates/core/src/react_types.rs` (two functions)
- Modify: `crates/core/src/resolver/named.rs` (step-6 suffix/name list)
- Create: `fixtures/rdt-compat/svg-icon.tsx`
- Test: `cargo test -p oxc-react-docgen-core --test snapshots`

- [ ] **Step 1: Add new fixture `fixtures/rdt-compat/svg-icon.tsx`**

```tsx
/**
 * Fixture for SVGAttributes / HTMLProps / ComponentRef patterns.
 *
 * These React namespace types must be recognized without producing
 * UnresolvableImport warnings, and SVGAttributes should map an HTML element
 * for notableInherited.
 */
import * as React from 'react'

export interface IconProps extends React.SVGAttributes<SVGSVGElement> {
  /** Icon size in pixels. */
  size?: number
  /** Icon color. */
  color?: string
}

/**
 * A generic SVG icon wrapper.
 */
export const Icon = React.forwardRef<SVGSVGElement, IconProps>(
  ({ size = 24, color = 'currentColor', ...props }, ref) => (
    <svg ref={ref} width={size} height={size} fill={color} {...props} />
  ),
)
Icon.displayName = 'Icon'

export interface BoxProps extends React.HTMLProps<HTMLDivElement> {
  /** Apply padding. */
  padded?: boolean
}

export const Box = React.forwardRef<HTMLDivElement, BoxProps>(({ padded, ...props }, ref) => (
  <div ref={ref} {...props} />
))
Box.displayName = 'Box'
```

- [ ] **Step 2: Read `react_types.rs` to find the exact insertion points**

Run:

```bash
grep -n "SVGAttributes\|HTMLProps\|ComponentRef\|JSXElementConstructor\|html_element_for\|is_react_builtin" crates/core/src/react_types.rs | head -30
```

Then open `crates/core/src/react_types.rs` and locate:

- `html_element_for()` — the match arms that map type name → element string
- `is_react_builtin()` — the list/match that returns `true` for known builtins

- [ ] **Step 3: Add to `html_element_for()` in `react_types.rs`**

After the existing `"AriaAttributes" => None` entry (or wherever the match ends), add:

```rust
"SVGAttributes" | "SVGProps" => None,      // SVG — no single element to pick
"HTMLProps" => Some("div"),                 // generic HTML props → div
```

The `None` return from `html_element_for` means "we recognize this is an HTMLAttributes- family type, but don't inject a specific element's notable attrs." That's correct for SVG.

- [ ] **Step 4: Add to `is_react_builtin()` in `react_types.rs`**

In the list of React builtin names (FC, ComponentType, etc.), add:

```rust
"ComponentRef" | "JSXElementConstructor" | "SVGAttributes" | "SVGProps" | "HTMLProps"
```

These are terminal React types — they provide no extractable props in our structural analysis; they should be composed/inherited, not resolved further.

- [ ] **Step 5: Add to step-6 silent list in `resolver/named.rs`**

Find the step-6 block in `named.rs` that checks known-harmless type names. Add:

```rust
|| bare == "SVGAttributes"
|| bare == "SVGProps"
|| bare == "HTMLProps"
|| bare == "ComponentRef"
|| bare == "JSXElementConstructor"
```

Or wherever the suffix/name matching is done. This prevents UnresolvableImport from firing when these are referenced from DTS files we can't follow.

- [ ] **Step 6: Compile and run tests**

```bash
cargo build -p oxc-react-docgen-core 2>&1 | head -20
cargo test -p oxc-react-docgen-core --test snapshots 2>&1 | tail -20
```

Expected: new snapshot for `svg-icon.tsx` is missing (snapshot tests will fail because the new fixture has no snapshot yet). That's fine — will be regenerated in Task 5.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/react_types.rs crates/core/src/resolver/named.rs fixtures/rdt-compat/svg-icon.tsx
git commit -m "feat: recognize SVGAttributes, HTMLProps, ComponentRef, JSXElementConstructor in React namespace"
```

---

## Task 3: Add `Readonly<T>` transparent wrapper in alias.rs

**Goal:** `type ReadonlyProps = Readonly<ButtonProps>` should pass through to `ButtonProps` rather than emitting zero props.

**Root cause:** `extractor/alias.rs:classify_type_alias` has arms for Omit, Pick, Partial, Required, then a `_ =>` wildcard that for "Readonly" creates `Passthrough { target: Named("Readonly") }`. That named type `"Readonly"` then hits `resolve_props_chain` step 1 (the utility-type silent list) and returns empty.

There are TWO places to handle Readonly:

1. `extractor/alias.rs` — teach `classify_type_alias` to create `Passthrough { target: inner_type }` (the base type)
2. `extractor/mod.rs:extract_type_name_from_type` — already handles `Readonly` at line 418! But only for prop type args, not for `type X = Readonly<Y>` aliases.

The fix in `alias.rs` is sufficient: match `"Readonly"` before `_ =>` and extract the inner type.

**Files:**

- Modify: `crates/core/src/extractor/alias.rs`
- Test: `cargo test -p oxc-react-docgen-core`

- [ ] **Step 1: Read `extractor/alias.rs` lines 1-80**

```bash
sed -n '1,80p' crates/core/src/extractor/alias.rs
```

Confirm the structure: Omit arm, Pick arm, Partial arm, Required arm, `_ =>` wildcard. Find the exact line where `_ =>` begins.

- [ ] **Step 2: Add `"Readonly"` arm before `_ =>`**

In `classify_type_alias`, before the `_ =>` wildcard, add:

```rust
"Readonly" => {
    let tp = tr.type_arguments.as_ref()?;
    let inner = tp.params.first()?;
    let (base_name, base_args) = self.extract_type_name_from_type(inner)?;
    let target = CollectedType::Named {
        name: base_name,
        args: base_args.into_iter().map(|a| CollectedType::Raw(a)).collect(),
    };
    Some(CollectedTypeAlias::Passthrough { target, file_path: fp })
}
```

This mirrors the Partial arm exactly — they both unwrap a single inner type and create a Passthrough alias pointing to the inner.

- [ ] **Step 3: Compile**

```bash
cargo build -p oxc-react-docgen-core 2>&1 | head -20
```

Expected: no errors. If `CollectedType::Named` has a different shape (check `types/collected.rs`), adjust accordingly — the `Named` variant takes `name: CompactString` and `args: Vec<CollectedType>`.

- [ ] **Step 4: Run unit tests**

```bash
cargo test -p oxc-react-docgen-core 2>&1 | grep -E "FAILED|ok|test result"
```

Expected: all unit tests pass. Snapshot tests may fail if any fixture uses `Readonly<T>` — that's fine, handled in Task 5.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/extractor/alias.rs
git commit -m "feat: handle Readonly<T> as transparent alias in extractor"
```

---

## Task 4: Fix inline `Pick<T,K>` / `Omit<T,K>` in extends position

**Goal:** `interface IconButtonProps extends Pick<ButtonBaseProps, 'disabled' | 'type' | 'form'>` should resolve `disabled`, `type`, `form` from `ButtonBaseProps`.

**Root cause:** When `classify_extends` processes `Pick<ButtonBaseProps, '...'>` it produces `ExtendsRef::SameFile { name: "Pick", type_args: ["ButtonBaseProps", "'disabled' | 'type' | 'form'"] }`. This calls `resolve_props_chain("Pick", type_args, ...)`. Step 1 of `resolve_props_chain` matches `"Pick"` and returns `ResolvedChain::default()` (empty) regardless of type_args.

The alias.rs Pick handler (which correctly resolves Pick) is only reachable from the `type_aliases` map — i.e., when the user writes `type X = Pick<T,K>`. When Pick appears in an `extends` clause directly, it bypasses the alias system entirely.

**Fix:** Add step 0.5 before step 1 in `resolve_props_chain`. If the type name is a utility type AND type_args is non-empty, construct a synthetic `CollectedTypeAlias` and route through `resolve_type_alias_chain` (which already handles Pick and Omit correctly).

**Files:**

- Modify: `crates/core/src/resolver/chain.rs`
- Modify: `crates/core/src/resolver/alias.rs` (if Pick/Omit parsing needs a raw-string helper)
- Test: `cargo test -p oxc-react-docgen-core --test snapshots`

- [ ] **Step 1: Read `resolver/chain.rs` lines 1-80 to confirm step 1 location**

```bash
sed -n '1,80p' crates/core/src/resolver/chain.rs
```

Confirm the `// ── Step 1` comment and its `matches!()` block. Note the exact line where step 1 begins.

- [ ] **Step 2: Read `resolver/alias.rs` to confirm Pick arm shape**

```bash
sed -n '1,80p' crates/core/src/resolver/alias.rs
```

Find the `CollectedTypeAlias::Pick { base, picked_keys, file_path }` arm. Note the field names — `base` is a `CollectedType`, `picked_keys` is a `Vec<CompactString>`.

- [ ] **Step 3: Add helper `parse_string_union_keys` to `resolver/chain.rs`**

This helper converts the raw type_args[1] string (e.g., `"'disabled' | 'type' | 'form'"`) into a `Vec<String>`. Add it as a private fn near the bottom of `chain.rs`:

```rust
fn parse_string_union_keys(raw: &str) -> Vec<String> {
    raw.split('|')
        .map(|s| s.trim().trim_matches('\'').trim_matches('"').to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}
```

- [ ] **Step 4: Add step 0.5 before step 1 in `resolve_props_chain`**

`CollectedTypeAlias` field types (from `types/collected.rs`):

- `file_path: Utf8PathBuf` — construct with `Utf8PathBuf::from(consuming_file)`
- `picked_keys / omitted_keys: Vec<String>` — plain `String`, not `CompactString`
- `CollectedType::Named { name: CompactString, args: Vec<CollectedType> }`

Immediately before the `// ── Step 1` comment, add:

```rust
// ── Step 0.5: Inline utility type in extends position ────────────────────
// Pick/Omit/Partial/Readonly appearing directly in `extends Pick<T,K>`
// have non-empty type_args. Route through alias resolver (same logic as
// `type X = Pick<T,K>`) instead of falling into the step-1 silent no-op.
if !type_args.is_empty() {
    let fp = Utf8PathBuf::from(consuming_file);
    let synthetic = match type_name_bare {
        "Pick" if type_args.len() >= 2 => {
            let base = CollectedType::Named {
                name: type_args[0].as_str().into(),
                args: vec![],
            };
            Some(CollectedTypeAlias::Pick {
                base,
                picked_keys: parse_string_union_keys(&type_args[1]),
                file_path: fp,
            })
        }
        "Omit" if type_args.len() >= 2 => {
            let base = CollectedType::Named {
                name: type_args[0].as_str().into(),
                args: vec![],
            };
            Some(CollectedTypeAlias::Omit {
                base,
                omitted_keys: parse_string_union_keys(&type_args[1]),
                file_path: fp,
            })
        }
        "Partial" if type_args.len() >= 1 => {
            let base = CollectedType::Named {
                name: type_args[0].as_str().into(),
                args: vec![],
            };
            Some(CollectedTypeAlias::Partial { base, file_path: fp })
        }
        "Readonly" if type_args.len() >= 1 => {
            let base = CollectedType::Named {
                name: type_args[0].as_str().into(),
                args: vec![],
            };
            Some(CollectedTypeAlias::Passthrough { target: base, file_path: fp })
        }
        _ => None,
    };
    if let Some(alias) = synthetic {
        return resolve_type_alias_chain(&alias, consuming_file, mapping, ctx, state, depth);
    }
}
```

Also add `use camino::Utf8PathBuf;` to the imports at the top of `chain.rs` if not already present — check with `grep "Utf8PathBuf" crates/core/src/resolver/chain.rs`.

- [ ] **Step 5: Compile**

```bash
cargo build -p oxc-react-docgen-core 2>&1 | head -30
```

If field names don't match, fix them. The key invariant: we're constructing the same `CollectedTypeAlias` shape that `extractor/alias.rs` produces for `type X = Pick<T,K>`.

- [ ] **Step 6: Run tests**

```bash
cargo test -p oxc-react-docgen-core 2>&1 | grep -E "FAILED|ok|test result"
```

Expected: unit tests pass. Snapshot tests will fail because pick-source.tsx will now have disabled/type/form props. That's correct — handled in Task 5.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/resolver/chain.rs
git commit -m "feat: resolve Pick/Omit/Partial/Readonly in extends position (step 0.5)"
```

---

## Task 5: Regenerate all snapshots and update coverage matrix

**Goal:** All snapshot tests pass; `docs/rdt-coverage.md` is updated to reflect fixed bugs.

**Files:**

- Modify: `crates/core/tests/snapshots/snapshots__snapshot_rdt_compat.snap`
- Modify: `crates/core/tests/snapshots/` (new svg-icon entry)
- Modify: `docs/rdt-coverage.md`

- [ ] **Step 1: Regenerate snapshots**

```bash
INSTA_UPDATE=always cargo test -p oxc-react-docgen-core --test snapshots 2>&1 | tail -30
```

Expected: all tests pass and snapshots are written.

- [ ] **Step 2: Verify key behavioral changes in updated snapshot**

Check `crates/core/tests/snapshots/snapshots__snapshot_rdt_compat.snap`:

```bash
grep -A3 "onValueChange" crates/core/tests/snapshots/snapshots__snapshot_rdt_compat.snap
```

Expected: `eventType: "string"` (was `"..."` before)

```bash
grep -A2 "disabled\|type.*form" crates/core/tests/snapshots/snapshots__snapshot_rdt_compat.snap | head -20
```

Expected: `disabled`, `type`, `form` now appear under `IconButton` props with `parent.name: "ButtonBaseProps"`

```bash
grep "Accordion\|accordion" crates/core/tests/snapshots/snapshots__snapshot_rdt_compat.snap
```

Note: Accordion may still be absent (union-of-interfaces root props bug is NOT fixed in this plan). That's expected and documented.

- [ ] **Step 3: Run full test suite**

```bash
cargo test -p oxc-react-docgen-core 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 4: Update `docs/rdt-coverage.md`**

In the "Known bugs" section, update:

- `Bug: method-shorthand handler loses param type` → mark as **FIXED** (2026-06-28), link to commit
- `Bug: Pick<T, Keys> not resolved even for source types` → mark as **FIXED** (2026-06-28), link to commit

In the "Component patterns" table, update:

- `Method-shorthand handler (shorthand form)` row: change `⚠️ BUG` to `✅` and update "Covered by" to `rdt-compat/controlled`
- `Pick<SourceInterface, Keys>` row: change `⚠️ BUG` to `✅` and update "Covered by" to `rdt-compat/pick-source`
- Add new row for `SVGAttributes` → `✅ rdt-compat/svg-icon`

In the "Known gaps summary" table, add new ✅ done rows:

- TSMethodSignature param extraction
- React namespace (SVGAttributes, HTMLProps, etc.)
- `Readonly<T>` transparent wrapper
- Inline `Pick<T,K>` in extends position

Add remaining unfixed items to a "Remaining structural gaps" section (not deferred/type-checker):

- Union-of-interfaces as root props type (silent skip, no diagnostic)
- Component description set to last prop's JSDoc

- [ ] **Step 5: Commit everything**

```bash
git add crates/core/tests/snapshots/ fixtures/rdt-compat/svg-icon.tsx docs/rdt-coverage.md
git commit -m "test: regenerate snapshots after structural gap fixes; update coverage matrix"
```

---

## Task 6: Create type-checker integration doc reference in coverage matrix

This task just links the already-written `docs/type-checker-integration.md` from the coverage matrix and STATUS doc so future readers know where to find the deferred work.

**Files:**

- Modify: `docs/rdt-coverage.md` (add section at bottom)
- Modify: `docs/09-STATUS.md` (add reference under known gaps)

- [ ] **Step 1: Add section to `docs/rdt-coverage.md`**

At the very bottom of `docs/rdt-coverage.md`, add:

```markdown
***

## Deferred gaps (require Corsa / TypeScript 7.1 type checker)

The following gaps cannot be fixed with structural AST analysis alone. See `docs/type-checker-integration.md` for the full plan, timeline, and integration architecture.

| Gap | Why deferred |
| --- | --- |
| Generic parameter substitution (`List<T>` where T from call site) | Requires `checker.getTypeArguments()` — Corsa API |
| Conditional types (`T extends U ? A : B`) | Already emitted as `opaque` — correct current behavior |
| Mapped types (`{ [K in keyof T]: … }`) | Already emitted as `opaque` — correct current behavior |
| `typeof expr` multi-level depth | `typeof Primitive.button` not followed to inferred type |
| Multi-file generic propagation | `ComponentProps<typeof Button>` across files |

**Target:** TypeScript 7.1 + Corsa API stable (est. Q1 2027) **Feature flag:** `--features=type-checker` (will not affect default builds)
```

- [ ] **Step 2: Add reference to `docs/09-STATUS.md`**

Find the "Known gaps" or "Open issues" section in `09-STATUS.md` and add:

```markdown
For the five structural gaps fixed in Phase X and the deferred type-checker gaps, see `docs/rdt-coverage.md` and `docs/type-checker-integration.md`.
```

- [ ] **Step 3: Commit**

```bash
git add docs/rdt-coverage.md docs/09-STATUS.md
git commit -m "docs: link type-checker-integration.md from coverage matrix and STATUS"
```

---

## Verification

After all tasks complete, run:

```bash
cargo test -p oxc-react-docgen-core
cargo clippy -p oxc-react-docgen-core -- -D warnings
```

Both should produce zero failures and zero warnings.

Check that these snapshot assertions hold:

1. `controlled.tsx` → `onValueChange` has `eventType: "string"` (not `"..."`)
2. `pick-source.tsx` → `IconButton` has `disabled`, `type`, `form` props
3. `svg-icon.tsx` → `Icon` has `size` and `color` props; no `UnresolvableImport` diagnostics
4. All existing fixtures still produce the same output (no regressions)
