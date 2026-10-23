# RDT spec coverage matrix

Maps each react-docgen-typescript (RDT) output field and component pattern to the fixtures that exercise it. Run `cargo test -p caliper-docgen-core --test snapshots` to validate the original 8 snapshot fixtures. The 8 real-world library fixtures added since (fluentui, base-ui, antd, ark-ui, zendesk-garden, day-picker, headlessui, blueprint) aren't snapshot-tested — validate them via `pnpm --filter caliper-docgen-validate compare:all`.

---

## PropType kinds

Each `PropType` variant the resolver can emit; whether any fixture produces it.

| Kind | Covered by |
| --- | --- |
| `string` | all fixtures |
| `number` | all fixtures |
| `boolean` | all fixtures |
| `null` | mui, shadcn |
| `unknown` | mui |
| `reactNode` | chakra, mantine, mui, react-aria |
| `cssProperties` | chakra, mui, react-aria, shadcn |
| `elementType` | chakra, mantine, mui |
| `stringLiteral` | chakra, mantine, mui, react-aria, shadcn |
| `union` | chakra, mantine, mui, react-aria, shadcn, **day-picker** (7-way discriminated) |
| `named` | chakra, mantine, mui, react-aria |
| `function` | all fixtures; every parameter with its real name, and the real return type (`(renderProps: ButtonRenderProps) => ReactNode`), verified against **headlessui**, **ark-ui**, **react-aria** |
| `ref` | mantine, mui |
| `object` | mantine, **rdt-compat/types** (`raw_string()` renders real fields — `{ key: Type; key2?: Type2 }` — not the literal placeholder `"object"`) |
| `literalUnion` | mantine, shadcn, **panda** (after defineRecipe arg-index fix) |
| `array` | **rdt-compat/types** (`string[]` syntax), **day-picker** (`readonly string[]` type-operator form) |
| `tuple` | **rdt-compat/types** (`[number, number]`; `raw_string()` renders real elements, not the placeholder `"tuple"`) |
| `numberLiteral` | **rdt-compat/types** (`1 \| 2 \| 4 \| 8`) |
| `boolLiteral` | **rdt-compat/types** (`true` literal type) |
| `undefined` | **rdt-compat/types** (explicit `?: undefined`) |
| `sxProps` | **rdt-compat/types** (unresolved `SxProps` ref → known-pattern shortcut) |
| `htmlAttributes` | **rdt-compat/types** (`ComponentPropsWithoutRef<'button'>` as prop value); real per-element attribute resolution (not just this metadata layer) — see [HtmlAttributeMode](#htmlattributemode-curated--full--none) below |
| `intersection` | **rdt-compat/types** (`CSSProperties & { '--accent': string }`) |
| `opaque` | **rdt-compat/types** (inline conditional type `T extends A ? B : C`) |
| `void` | as a `function`'s return type (`() => void`); standalone `void` never appears |
| `never` | ❌ not covered — no real-world prop has type `never` |
| `any` | ❌ not covered — `any` suppressed by TypeScript `strict` in fixtures |

> **Note on `void` / `never`:** These kinds exist in the type system for completeness but are not realistic prop types. A `void` return type appears inside a `function`, and `never` typically indicates a broken discriminant. No fixture is planned for either.

---

## ComponentEntry fields

| Field | Status | Covered by |
| --- | --- | --- |
| `displayName` | ✅ | all fixtures |
| `filePath` | ✅ | all fixtures |
| `description` (non-empty) | ✅ | mui Button, panda button |
| `description` (component JSDoc, no prop leak) | ✅ | fixed — see [Fixed: component description leak](#fixed-component-description-set-to-last-props-jsdoc) |
| `props` | ✅ | all fixtures |
| `inheritance` (non-empty) | ✅ | all fixtures |
| `notableInherited` | ✅ (curated mode only) | all fixtures — see [HtmlAttributeMode](#htmlattributemode-curated--full--none) |
| `discriminantProp: null` | ✅ | all fixtures; also correctly `null` for a _double_-discriminated union where no single field disambiguates every variant — **day-picker** (`mode` repeats across `PropsSingle`/`PropsSingleRequired`, only jointly unique with `required`) |
| `discriminantProp: "variant"` | ✅ | mui TextField |
| `discriminantProp` through an intersection-wrapped union (`Base & (A\|B\|C)`) | ✅ | **day-picker** — see [Fixed: intersection-wrapped discriminated unions](#fixed-discriminated-unions-wrapped-in-an-intersection) |
| `composes` (non-empty) | ✅ | mantine, shadcn, chakra, react-aria |
| `tags` (component-level, non-empty) | ✅ | **rdt-compat/jsdoc** (`@description`, `@default`, `@deprecated`, `@internal`) |
| `methods: []` | ✅ | all fixtures (always empty; class components not in scope) |

---

## ParsedProp fields

| Field | Status | Covered by |
| --- | --- | --- |
| `name` | ✅ | all fixtures |
| `type` | ✅ | all fixtures |
| `required: true` | ✅ | all fixtures |
| `required: false` | ✅ | all fixtures |
| `defaultValue: null` | ✅ | all fixtures |
| `defaultValue.value` from JSDoc `@default` | ✅ | mui, mantine |
| `defaultValue.value` from code destructor | ✅ | mui (`{ color = 'primary' }`) |
| `defaultValue.computed: false` | ✅ | mui, mantine |
| `defaultValue.computed: true` | ❌ | not covered — no fixture uses computed defaults |
| `description` (non-empty) | ✅ | mui, radix |
| `tags.default` | ✅ | mui, mantine |
| `tags.deprecated` | ✅ | mantine; correctly scoped to only the annotated prop, not bled onto siblings — **antd** (see [Fixed: JSDoc tag bleed](#fixed-jsdoc-tag-bleeding-onto-sibling-props)) |
| `tags.see` | ✅ | mui, mantine |
| `tags.example` | ✅ | mantine |
| `tags.since` | ✅ | **rdt-compat/jsdoc** (`@since` on `header` prop) |
| `parent` (non-null) | ✅ | all fixtures |
| `parent: null` | ✅ | shadcn (CVA-derived props) |
| `declarations` (non-empty) | ✅ | all fixtures |
| `declarations: []` | ✅ | shadcn |

---

## Component patterns

| Pattern | Status | Covered by |
| --- | --- | --- |
| `React.forwardRef` | ✅ | shadcn, panda, mantine, react-aria |
| `React.forwardRef(...) as X` (trailing cast) | ✅ | **fluentui**, **ark-ui** — see [Fixed: forwardRef as-cast](#fixed-forwardref-wrapped-in-an-as-cast) |
| Same-file custom wrapper + bare identifier (`customWrapper(NamedFn) as X`) | ✅ | **headlessui** — see [Fixed: identifier-wrapped naming](#fixed-identifier-wrapped-components-renamed-to-export-binding) |
| `React.memo` | ✅ | **rdt-compat/memo** |
| CVA variants (`cva()`) | ✅ | shadcn |
| CVA enum output in `enums` | ✅ | shadcn |
| PandaCSS `defineRecipe()` variants | ✅ | **panda** (fixed: arg-index bug corrected 2026-06-28) |
| PandaCSS `defineSlotRecipe()` | ⚠️ | see [Note: slot recipes](#note-slot-recipes-partial-support) |
| Discriminated union (per-variant props) | ✅ | mui TextField, **day-picker** (7-way, double-discriminated) |
| Union-of-interfaces as root props type | ✅ | fixed — see [Fixed: union-of-interfaces](#fixed-union-of-interfaces-root-props-type-silently-skipped-the-component) |
| `asChild` slot pattern | ✅ | radix, shadcn, panda |
| Classic `as="button"` polymorphism (not `asChild`, not render-prop) | ✅ | **headlessui** |
| Intersection-based prop inheritance | ✅ | chakra, mantine, mui |
| `Omit<T, Keys>` on inherited props | ✅ | chakra, mantine, mui |
| Two `Partial<X>` mixins on one interface | ✅ | **blueprint** — see [Fixed: Partial\<X\> cycle-key collision](#fixed-partialx-cycle-detection-key-collision) |
| `Pick<SourceInterface, Keys>` | ✅ | fixed — see [Fixed: Pick not resolved](#fixed-pickt-keys-not-resolved-even-for-source-types) |
| User-defined generic type alias substitution (`type Assign<T,U> = Omit<T, keyof U> & U`) | ✅ | **ark-ui** — see [Fixed: generic type alias substitution](#fixed-user-defined-generic-type-alias-substitution) |
| Same-file TS `namespace X { type Props }` member reference | ✅ | **base-ui** — see [Fixed: namespace member resolution](#fixed-same-file-namespace-member-resolution) |
| Controlled/uncontrolled triple | ✅ | **rdt-compat/controlled** (value + defaultValue + onValueChange) |
| Method-shorthand handler (arrow form) | ✅ | **rdt-compat/controlled** (`onOpenChange?: (open: bool) => void` → correct `eventType`) |
| Method-shorthand handler (shorthand form) | ✅ | fixed — see [Fixed: method-shorthand](#fixed-method-shorthand-handler-losing-param-type) |
| JSDoc prop description | ✅ | radix, mui |
| JSDoc `@default` → `defaultValue` | ✅ | mui, mantine |
| JSDoc `@deprecated` on prop | ✅ | mantine, antd |
| JSDoc `@see` / `@example` on prop | ✅ | mui, mantine |
| JSDoc description on component | ✅ | mui Button, panda button |
| JSDoc tags on component (`@see`, `@since`, `@category`) | ✅ | **rdt-compat/jsdoc** |
| Render-prop children | ✅ | react-aria (`ButtonRenderProps`) |
| HTML element inheritance, curated | ✅ | all fixtures |
| HTML element inheritance, full real resolution | ✅ | shadcn, **blueprint** — see [HtmlAttributeMode](#htmlattributemode-curated--full--none) |
| `styled.X.attrs<T>()` (styled-components) | ❌ | **zendesk-garden** — neither this tool nor real RDT detects this pattern; a shared blind spot, not a competitive gap, not fixed |
| Template literal type → expanded literal union | ✅ | mantine `size?: MantineSize \| \`compact-${MantineSize}\``resolves to a`union`of two`literalUnion`s (`xs`/`sm`/.../`xl`and`compact-xs`/.../`compact-xl`), not left opaque |
| HOC pattern | ✅ | extractor unit test only (no snapshot fixture) |
| Generic component | ⚠️ partial | own generic type ALIAS substitution works (**ark-ui**); React's own builtin generics resolve by name only, never by substituted value (sufficient for docgen — see `docs/type-checker-integration.md`); component-level `<T,>()=>` type-param flow through a multi-file component tree not covered |
| `defaultProps` static assignment | ❌ | no fixture |
| Class component | ❌ | out of scope (modern React only) |

---

## ExtractionOutput top-level fields

| Field                       | Status | Covered by                                  |
| --------------------------- | ------ | ------------------------------------------- |
| `components` (non-empty)    | ✅     | all fixtures                                |
| `enums` (non-empty)         | ✅     | shadcn, **panda** (after defineRecipe fix)  |
| `diagnostics` (non-empty)   | ✅     | mantine, mui (UNRESOLVABLE_IMPORT warnings) |
| `stats.componentsExtracted` | ✅     | all fixtures                                |
| `stats.componentsSkipped`   | ✅     | all fixtures                                |
| `stats.filesParsed`         | ✅     | all fixtures                                |
| `stats.opaqueCount`         | ❌     | never non-zero in any snapshot              |

---

## HtmlAttributeMode: curated / full / none

`PipelineOptions.html_attributes` (CLI: `--html-attributes <curated|full|none>`; NAPI: `htmlAttributes: 'curated' | 'full' | 'none'`; config file: `htmlAttributes` in `docgen.config.ts`) controls how much of an inherited HTML element's attribute surface gets exposed:

- **Curated (default)** — unchanged original behavior: ~15-20 hand-picked, commonly-documented attributes per element (`onClick`, `disabled`, `aria-*`, etc.) synthesized into `notableInherited`.
- **Full** — actually resolves `@types/react`'s real `HTMLAttributes`/`AriaAttributes`/`DOMAttributes`/`<Element>HTMLAttributes` interface chain (merged into `GlobalSourceData` by the pipeline, looked up by the resolver like any other interface) and merges the real fields directly into `props` — matching how RDT flattens everything into one props map. Verified against a real button: 238 of ~250 real attributes resolve (the remainder is a narrower, separate gap — see below). Costs ~16ms once per `@types/react` version (cached the same way any other `.d.ts` is), not per extraction run.
- **None** — no inherited HTML attributes synthesized at all; own props only.

**Known residual gap, not fixed:** a handful of fields inside `@types/react`'s own interface chain (`AriaAttributes` referenced bare from within the same enclosing `declare namespace React {}` block, not through an explicit `React.` qualifier) don't resolve — same-namespace _sibling_ reference resolution is narrower than the namespace-_qualified_ reference resolution fixed for cross-file/explicit cases. Degrades gracefully (an `UNRESOLVABLE_IMPORT` diagnostic on that one field, not a crash or component loss).

---

## Known gaps summary

| Priority | Gap | Path to cover |
| --- | --- | --- |
| ✅ done | `opaque` kind | `fixtures/rdt-compat/types.tsx` (inline conditional) |
| ✅ done | `array` kind | `fixtures/rdt-compat/types.tsx` (`string[]`) |
| ✅ done | `tuple` kind | `fixtures/rdt-compat/types.tsx` (`[number, number]`) |
| ✅ done | `numberLiteral` kind | `fixtures/rdt-compat/types.tsx` (`1 \| 2 \| 4 \| 8`) |
| ✅ done | `sxProps` kind | `fixtures/rdt-compat/types.tsx` (unresolved `SxProps` ref) |
| ✅ done | `htmlAttributes` kind | `fixtures/rdt-compat/types.tsx` (`ComponentPropsWithoutRef<'button'>`) |
| ✅ done | `React.memo` wrapper | `fixtures/rdt-compat/memo.tsx` |
| ✅ done | `boolLiteral` kind | `fixtures/rdt-compat/types.tsx` (`true` literal) |
| ✅ done | `undefined` kind | `fixtures/rdt-compat/types.tsx` (explicit `?: undefined`) |
| ✅ done | `intersection` in prop value | `fixtures/rdt-compat/types.tsx` (`CSSProperties & { … }`) |
| ✅ done | Component-level JSDoc `tags` | `fixtures/rdt-compat/jsdoc.tsx` |
| ✅ done | `tags.since` on prop | `fixtures/rdt-compat/jsdoc.tsx` |
| ✅ done | PandaCSS `defineRecipe` arg-index bug | `interface.rs:77` fix; panda snapshot updated |
| ✅ done | Controlled/uncontrolled triple | `fixtures/rdt-compat/controlled.tsx` |
| ✅ done | `Pick<SourceInterface>` | `fixtures/rdt-compat/pick-source.tsx`; fixed, see below |
| ✅ done | Union-of-interfaces root props | `fixtures/rdt-compat/discriminated-union.tsx`; fixed, see below |
| ✅ done | Method-shorthand handler param type | `fixtures/rdt-compat/controlled.tsx`; fixed, see below |
| ✅ done | Component description leak from last prop's JSDoc | fixed, see below |
| ✅ done | `forwardRef(...) as X` trailing cast | `fixtures/fluentui`, `fixtures/ark-ui`; fixed, see below |
| ✅ done | Same-file TS namespace member resolution | `fixtures/base-ui`; fixed, see below |
| ✅ done | JSDoc `@tag` bleeding onto sibling props | `fixtures/antd`; fixed, see below |
| ✅ done | User-defined generic type alias substitution | `fixtures/ark-ui`; fixed, see below |
| ✅ done | `Partial<X>` cycle-detection key collision | `fixtures/blueprint`; fixed, see below |
| ✅ done | Discriminated union wrapped in an intersection | `fixtures/day-picker`; fixed, see below |
| ✅ done | Bare function-type alias silently dropped | `fixtures/day-picker`; fixed, see below |
| ✅ done | Identifier-wrapped component wrong naming | `fixtures/headlessui`; fixed, see below |
| ✅ done | `Tuple`/`Object` `raw_string()` placeholder strings | real content now rendered, see PropType kinds table |
| ✅ done | 4 missing `*EventHandler` builtin names | `ReactEventHandler`, `SubmitEventHandler`, `InputEventHandler`, `ToggleEventHandler` |
| ✅ done | `readonly X` / `unique X` type operators captured as raw text | now peeled transparently, matching `keyof`'s structured handling |
| ✅ done | Type alias union/intersection members resolved relative to caller's file, not the alias's own file | `fixtures/tanstack-table`; fixed, see below |
| ✅ done | Generic interface/alias's own type parameters flagged as unresolvable | `fixtures/tanstack-table`; fixed, see below |
| ✅ done | Named type-only imports from `react` (wrong file + bare-vs-namespace-qualified key) | `fixtures/react-resizable-panels`; fixed, see below |
| ✅ done | Indexed access into a generic interface's own field | `fixtures/react-final-form`; fixed, see below |
| ✅ done | Type aliases silently dropped for any unhandled body shape (generalized beyond the two prior special cases) | `fixtures/storybook-emotion`; fixed, see below |
| ✅ done | Entire file silently discarded when JSDoc prose contains unmatched brackets | real `typescript` package's own `lib.dom.d.ts`; fixed, see below |
| ✅ done | Indexed access into an ambient DOM interface not walking its `extends` chain | `fixtures/day-picker` (`HTMLDivElement["dir"/"nonce"/"title"/"lang"]`); fixed, see below |
| ✅ done | `(typeof X)[number]` on a flat `const X = [...] as const` array | `fixtures/antd` (`ButtonType`, `ButtonShape`, `ButtonHTMLType`, `ButtonVariantType`, `ButtonColorType`); fixed, see below |
| ✅ done | Static `X.defaultProps = {...}` assignment not read (only destructured defaults were) | no fixture ships a real static assignment today (`fixtures/blueprint/Table.tsx` was written around this exact gap — see its updated header comment); covered by unit + pipeline tests, fixed, see below |
| ✅ done | Bare passthrough identifier alias (`const Button = InternalCompoundedButton;`) not renaming the component | `fixtures/antd` (`Button` surfaced as `InternalCompoundedButton`); fixed, see below |
| N/A | `void` kind | standalone `void` not a real prop type |
| N/A | `never` kind | broken discriminant, not a real prop type |
| N/A | `any` kind | suppressed by `strict` mode |
| N/A | React 19 `ref`-as-prop (`function Button({ ref, ...props })`) | investigated, not a real gap — see below |
| N/A | Compound components (`Dialog.Trigger`) | investigated, not a real gap — see below |
| ❌ not fixed | `styled.X.attrs<T>()` component detection | `fixtures/zendesk-garden`; shared blind spot with real RDT, not a competitive gap |
| ❌ not fixed | `ComponentProps<typeof StyledButton>` where `StyledButton` is `@emotion/styled`'s `styled(tag, options)<T>(fn)` two-arg overload | `fixtures/storybook-emotion`; requires recognizing a new call-expression shape, capturing its curried generic type argument, and merging with the base element's real HTML attributes — a new library-specific pattern (comparable in scope to the existing `VariantProps`/cva shortcut), not a bug fix. Real RDT needs a type checker for this too |
| ❌ not fixed | Same-namespace sibling reference resolution for some `@types/react` internals (e.g. `EventHandler`, `TrustedHTML`) | narrower residual case not covered by the bare/`React.`-qualified key fallback; degrades gracefully |

---

## Fixed bugs (previously tracked here as open)

### Fixed: method-shorthand handler losing param type

**Fixture:** `fixtures/rdt-compat/controlled.tsx`

Previously: `onValueChange?(value: string): void` (TSMethodSignature form) emitted `eventType: "..."` instead of `eventType: "string"`, while the arrow-function form correctly emitted `eventType: "boolean"`. Root cause: the extractor's method-signature handling didn't extract parameter types the same way as property-signature handling. Fixed — confirmed live: `onValueChange` now correctly resolves to `(value: string) => void`.

---

### Fixed: `Pick<T, Keys>` not resolved even for source types

**Fixture:** `fixtures/rdt-compat/pick-source.tsx`

Previously: `interface IconButtonProps extends Pick<ButtonBaseProps, 'disabled' | 'type' | 'form'>` lost the picked props entirely — only `icon`/`label` (the interface's own props) appeared. Fixed — confirmed live: `IconButton` now correctly includes `disabled`, `form`, `type` alongside `icon`/`label`.

---

### Fixed: union-of-interfaces root props type silently skipped the component

**Fixture:** `fixtures/rdt-compat/discriminated-union.tsx`

Previously: `Accordion` with props type `AccordionSingleProps | AccordionMultipleProps` was completely absent from the output, no diagnostic. Fixed — confirmed live: `Accordion` now resolves with all 5 real props.

---

### Fixed: component description set to last prop's JSDoc

**Fixtures:** `fixtures/rdt-compat/controlled.tsx`, `pick-source.tsx`, `memo.tsx`

Previously: when a component had no JSDoc of its own, `ComponentEntry.description` picked up the last prop's JSDoc instead of staying empty (the extractor's proximity-based `find_jsdoc` scan finding the wrong nearest comment). Fixed — confirmed live: `Select`/`IconButton`/`Avatar` all correctly show `description: ""`.

---

### Fixed: forwardRef wrapped in an `as`-cast

**Fixtures:** `fixtures/fluentui/Button.tsx`, `fixtures/ark-ui/Select.tsx`

Fluent UI's real Button (and Ark UI's real Select) are authored as `const X: SomeWrapper<Props> = React.forwardRef((props, ref) => {...}) as SomeWrapper<Props>` — neither `try_fc_annotation` (the wrapper type name wasn't recognized) nor `try_forward_ref`/`try_hoc_wrapped` (both required the initializer to be a `CallExpression` directly, not a `TSAsExpression` wrapping one) detected this; the component was silently invisible. Fixed by recognizing `ForwardRefComponent<P>` as a wrapper annotation and peeling `as`-casts before matching the call expression underneath. Confirmed: Fluent's Button 0 → 8 props.

---

### Fixed: same-file namespace member resolution

**Fixture:** `fixtures/base-ui/MenuRoot.tsx`, `MenuTrigger.tsx`

Base UI's real pattern is `namespace MenuRoot { export type Props<Payload> = ... }`, referenced elsewhere in the same file as `MenuRoot.Props`. Storage keyed on the bare member name while the resolver looked up the fully-qualified dotted name — same-file namespace member references could never resolve. Fixed by tracking an enclosing-namespace stack during extraction and qualifying storage keys to match. Confirmed: MenuRoot 0 → 15 props (exact match with real RDT), MenuTrigger invisible → 11 props.

---

### Fixed: JSDoc `@tag` bleeding onto sibling props

**Fixture:** `fixtures/antd/Button.tsx`

`find_jsdoc` (description) tracked consumed comment spans so two elements never share one description; `extract_jsdoc_tags` (the `@tag` map) ran a separate scan with no consumed-tracking at all, so a `@deprecated` tag correctly claimed by one prop was still "found" and inherited by a later sibling with no JSDoc of its own. Fixed by merging both lookups into one consuming pass. Confirmed: only `iconPosition` carries `@deprecated` now, not `iconPlacement`/`shape`/`size`/`disabled`.

---

### Fixed: user-defined generic type alias substitution

**Fixture:** `fixtures/ark-ui/Select.tsx`

`type Assign<T, U> = Omit<T, keyof U> & U` used with concrete call-site arguments (Ark UI's real `SelectRootProps<T> = Assign<HTMLProps<'div'>, SelectRootBaseProps<T>>`) never substituted `T`/`U` — type alias declarations didn't record their own declared parameters, and call-site type arguments were computed but discarded before alias resolution ran. `Omit<T, keyof U>`'s `keyof U` was also silently treated as an empty key list. Fixed via a structural walk-and-replace substitution engine (no type inference — real field names for `keyof U` come from resolving `U` as its own props chain). Confirmed: SelectRoot 0 → 36 real props via the actual, unmodified upstream pattern.

---

### Fixed: `Partial<X>` cycle-detection key collision

**Fixture:** `fixtures/blueprint/Table.tsx`

`TableProps extends Partial<RowHeights>, Partial<ColumnWidths>` — the resolver's cycle-detection visited-key was built from the bare type name alone (`"Partial"`), not its type arguments, so the second `Partial<X>` extends target collided with the first's cycle-guard entry and silently resolved to nothing, with zero diagnostic. Fixed by folding type arguments into the key. Confirmed: Table 45 → 48 props, exact match with real RDT.

---

### Fixed: discriminated unions wrapped in an intersection

**Fixture:** `fixtures/day-picker/DayPicker.tsx`

Discriminant detection only ran when a type alias's RHS was _directly_ a union (`type X = A | B`). Real Day Picker's `DayPickerProps = PropsBase & (7-way union)` — an intersection whose union member fell into a naive per-member merge instead, keeping only whichever branch's type was seen first per prop instead of unioning across all branches. Fixed by having the intersection-nested-union case delegate to the same discriminant-merge logic a direct union alias uses. Confirmed: `selected`'s type now correctly merges `Date | Date[] | DateRange` across all 7 branches (previously collapsed to bare `Date`). Also confirmed (and this is _correct_, not a remaining bug): `discriminantProp` stays `null` for Day Picker's real union, because `mode` repeats the same value across `PropsSingle`/`PropsSingleRequired` — it's only jointly unique with a second field (`required`), so no single field actually identifies the variant.

---

### Fixed: bare function-type alias silently dropped

**Fixture:** `fixtures/day-picker/props.ts`

`type OnSelectHandler<T> = (selected: T, ...) => void` — a bare function type as an alias body — fell through `classify_type_alias`'s catch-all and vanished from `type_aliases` with no diagnostic, same failure mode as the already-fixed inline-object-literal case. Fixed by routing `TSFunctionType` through the same `Passthrough` wrapping `TSTypeLiteral` already used. Confirmed: `OnSelectHandler` no longer triggers a "cannot resolve" warning.

---

### Fixed: identifier-wrapped components renamed to export binding

**Fixture:** `fixtures/headlessui/Listbox.tsx`

Headless UI's real Listbox family: standalone top-level function declarations (`function ButtonFn(props, ref)`) wrapped by a library-defined `forwardRefWithAs` (not React's own `forwardRef`) and reassigned (`export let ListboxButton = forwardRefWithAs(ButtonFn) as X`). `ButtonFn` was already independently detected as its own component (a valid top-level PascalCase function with a typed first param) under the wrong, inner-implementation-only name — neither `try_forward_ref` (the callee isn't `React.forwardRef`) nor `try_hoc_wrapped` (the argument is a bare identifier, not an inline function) recognized the wrapping assignment. Fixed the same way the existing `displayName`-scan already handles a similar after-the-fact rename: when a variable's initializer is (after unwrapping any `as` cast) a call whose sole argument is a bare identifier matching an already-collected component, rename that mapping to the outer binding. Confirmed: all three components (`Listbox`, `ListboxButton`, `ListboxOption`) now report their real export names, matching real RDT.

---

### Fixed: type alias union/intersection members resolved relative to the wrong file

**Fixture:** `fixtures/tanstack-table/types.ts`

`ColumnDef<TData, TValue> = DisplayColumnDef<...> | GroupColumnDef<...> | AccessorColumnDef<...>` — a union whose members are same-file siblings of the alias, imported cross-file into `data-table.tsx` as just `ColumnDef`. `resolve_type_alias_type` forwarded the _caller's_ `consuming_file` into the recursive member resolution instead of the alias's own declaring file, so `DisplayColumnDef` etc. were looked up relative to `data-table.tsx` (which never imports them directly) and spuriously flagged as unresolvable, even though the actual `PropType::Named` output was already correct. Fixed by giving `CollectedTypeAlias` a `file_path()` accessor and using it instead of the passed-in file for every recursive call.

### Fixed: generic interface/alias's own type parameters flagged as unresolvable

**Fixture:** `fixtures/tanstack-table/data-table.tsx`

`DataTableProps<TData, TValue>` referencing its own `TData`/`TValue` in its body (`columns: ColumnDef<TData, TValue>[]`) had every such reference warned as "cannot resolve — will appear as opaque", even though a bare generic placeholder is the objectively correct, expected output. The resolver had no concept of a type's own declared parameters. Added `interface_type_params` (mirroring the existing `type_alias_params`) and a `ResolveState.in_scope_type_params` set populated when entering a generic interface/alias body; `resolve_named` checks it before warning. 234 → 173 diagnostics across all fixtures.

### Fixed: named type-only imports from `react` resolved to the wrong file/key

**Fixture:** `fixtures/react-resizable-panels/types.ts`

`import type { X } from "react"` (any `X` not special-cased via `html_element_for`) failed for two independent reasons: (1) `react`'s own `package.json` has no `"types"` field/condition — its real declarations live in the separate `@types/react` package — so the general import resolver landed on `index.js`; (2) `@types/react` declares everything inside `declare namespace React { ... }`, so even the right file's declarations are keyed `"React.X"`, not bare `"X"`. Fixed resolver-wide: `resolve_import_specifier` now tries `resolve_dts` + the `@types/<package>` fallback for bare specifiers (reusing the logic already proven for `HtmlAttributeMode::Full`), and new `lookup_interface`/`lookup_type_alias` helpers try the bare key before the `React.`-qualified one, replacing direct map lookups at every interface/alias resolution site.

### Fixed: indexed access into a generic interface's own field

**Fixture:** `fixtures/react-final-form/types.ts`

`RenderableProps<FieldRenderProps<FieldValue, T>>["children"]` (the real "children as a render-function-or-ReactNode union" pattern) degraded to `Opaque` — `resolve_indexed_access`'s only fallback resolved the object type and checked for `PropType::Object`, but an interface always resolves to a bare `PropType::Named` at the type level (never expanded there), so it never matched. Added a path that looks the field up directly on the interface's declaration and substitutes its declared type parameters with the caller's concrete arguments, reusing the existing generic-alias substitution machinery. Both indexed-access fields in this fixture (`children`, `component`) now resolve to real structured types with the correct substituted argument threaded through, zero diagnostics.

### Fixed: type aliases silently dropped for any unhandled body shape

**Fixture:** `fixtures/storybook-emotion/Button.tsx`

`type API_KeyCollection = string[]` — found while investigating the fixture below. `classify_type_alias`'s catch-all was `_ => None`: any alias body shape without a dedicated match arm vanished from `type_aliases` entirely, no diagnostic, same failure mode already fixed twice this session for `TSTypeLiteral` and `TSFunctionType` individually. Generalized the catch-all itself to `Passthrough`-wrap whatever `ts_type_to_collected` already produces, rather than adding a third narrow special case. 171 → 153 diagnostics across all fixtures (curated), 224 → 206 (full) — fixed the same silent-drop in other fixtures too, not just this one.

### Fixed: entire file silently discarded when JSDoc prose contains unmatched brackets

**Fixture:** the real `typescript` npm package's own `lib.dom.d.ts` (found while extending native-global resolution to DOM ambient types for the fix below).

`max_bracket_nesting_depth` — the pre-parse guard that bails out before handing pathologically nested types to `oxc_parser`'s recursive-descent parser — counted `(`/`{`/`[`/`)`/`}`/`]` byte-by-byte with no awareness of comments or string/template literals. Real-world `.d.ts` prose routinely contains unbalanced brackets (MDN-scraped JSDoc in `lib.dom.d.ts` has artifacts like `... MISSING: RFC(5646, '...')].`, a stray `]` with no opening `[`, roughly 2000 times); each one drove the running depth negative, and once negative enough, the next legitimate bracket in real code still left it negative — casting that negative `i64` to `usize` wrapped to ~`u64::MAX`, tripping the "exceeds maximum nesting depth" guard and silently discarding the entire 1.8MB file (0 interfaces extracted, no diagnostic reaching the user beyond a generic skip message). Rewrote the scan to skip `//`/`/* */` comments and `'...'`/`"..."`/`` `...` `` literals entirely rather than counting their bracket content, and switched `depth` to `usize` with `saturating_sub` so it can never go negative regardless of any remaining edge case. Confirmed against the real installed `lib.dom.d.ts`: 0 → thousands of interfaces extracted, no more false-positive `ExcessiveNesting` diagnostic.

### Fixed: indexed access into an ambient DOM interface not walking its `extends` chain

**Fixture:** `fixtures/day-picker/props.ts`

`dir?: HTMLDivElement["dir"]` (also `nonce`, `title`, `lang`) degraded to `Opaque` even after `lib.dom.d.ts` became parseable (previous fix). Two compounding gaps: `resolve_indexed_access`'s interface lookup only checked the consuming file's own imports/declarations, never TypeScript's own ambient lib files, so `HTMLDivElement` (never imported — it's a global) was never found at all; and even once found, `dir`/`nonce`/`title`/`lang` aren't declared directly on `HTMLDivElement` — they're inherited via `extends HTMLElement` (and, for `nonce`, `HTMLElement extends ... HTMLOrSVGElement`) — and the existing field lookup only checked an interface's own `props`, never its extends chain. Added `lookup_interface_including_ambient` (checks `ctx.ambient_global_files` as a fallback) and a small recursive ancestor-chain search over `ExtendsRef::SameFile` entries. All 4 props now resolve to `string`; `day-picker/DayPicker` prop count now matches react-docgen-typescript exactly (68/68).

### Fixed: `(typeof X)[number]` on a flat `const X = [...] as const` array

**Fixture:** `fixtures/antd/buttonHelpers.tsx`

```ts
const _ButtonTypes = ['default', 'primary', 'dashed', 'link', 'text'] as const
export type ButtonType = (typeof _ButtonTypes)[number]
```

Two gaps, neither previously covered: nothing captured a plain array-literal `const` (only cva/tv's `variants` object shape and const-object enums), and indexed access never handled a `[number]` key — `key` there is `CollectedType::Number` (the `number` keyword type), not a string literal, so the existing string-literal-keyed lookup always missed it. Added `SourceData::const_arrays` (deliberately separate from `enums`, which has per-entry variant names and is surfaced directly in the public output — a plain array has neither), populated by a new `try_collect_const_array` mirroring `try_collect_const_enum`'s shape but for `ArrayExpression`. Added a matching `const_array_bare_index` on `ResolutionContext` (same O(1) pattern as `enum_bare_index`) and a new branch in `resolve_indexed_access` that builds a `LiteralUnion` from the array's values when `obj` is `TypeOf` and `key` is `Number`. All 5 exported unions in the fixture (`ButtonType`, `ButtonShape`, `ButtonHTMLType`, `ButtonVariantType`, `ButtonColorType`) now resolve correctly; the 10 associated diagnostics are gone.

`ButtonColorType`'s source array uses a spread (`['default', 'primary', 'danger', ...PresetColors]`) — the spread element is silently skipped (no expression to read a literal from), so that one union resolves with 3 of its real ~19 members. A known, honest partial result, not a crash or a wrong type; matches how cva/tv variant extraction already treats constructs it can't statically read.

### Fixed: static `X.defaultProps = {...}` assignment not read

`ComponentMapping::param_defaults` already had a `DefaultSource::DefaultProps` variant — designed for but never wired up. Only destructured defaults (`function Button({ size = 'md' })`, via `extract_param_defaults`) were ever populated. Real components (MUI ships this on several) also set defaults via a static assignment after the declaration:

```ts
function Button(props: ButtonProps) { ... }
Button.defaultProps = { size: 'md' };
```

Added `try_scan_default_props`, mirroring the existing `try_scan_display_name`'s shape (same "find the `ExpressionStatement` assigning to `X.<prop>`, look up the matching `ComponentMapping`" pattern) — extracts each object property via `eval_expr_as_default`, now parameterized with a `DefaultSource` instead of hardcoding `Destructuring`, so both call sites tag their defaults correctly. The resolver's consumption side (`resolver/chain.rs`'s `mapping.param_defaults.get(&raw_prop.name)`) needed no changes — it was already source-agnostic, just never had anything to read from this path.

No fixture ships a real static `defaultProps` assignment today — `fixtures/blueprint/Table.tsx` had in fact been deliberately written _around_ this exact gap (its header comment said so explicitly). Comment updated; fixture coverage stays as destructured defaults since upstream Blueprint's `Table` is a class component and this fixture uses a function component by design (a separate, unrelated tradeoff — see the fixture's own comment). Covered instead by a dedicated extractor unit test and a pipeline end-to-end test proving the value reaches `ParsedProp.default_value`.

---

## Note: React 19 `ref`-as-prop — investigated, not a real gap

A `ReactVersion` option (since removed, because nothing in `crates/core` ever read it) was plumbed from the CLI down to `PipelineOptions`. That looked like a real gap — the original spec's worry was that `function Button({ ref, ...props }: ButtonProps)` (React 19's plain-prop `ref`, no `forwardRef`) wouldn't be recognized.

Tested directly against both real shapes:

```ts
// ref declared directly on the props interface
interface ButtonProps { label: string; ref?: Ref<HTMLButtonElement>; }
export function Button({ ref, ...props }: ButtonProps) { ... }

// ref merged in via intersection (the more common real-world shape)
type ButtonProps = BaseProps & { ref?: Ref<HTMLButtonElement> };
export function Button({ ref, ...props }: ButtonProps) { ... }
```

Both already extract `ref` correctly as `{ kind: 'ref', element: 'HTMLButtonElement' }`, no special-casing involved — the extractor doesn't care that the member happens to be named `ref`; it walks whatever the props type structurally contains, same as any other member, regardless of React version. The one case this doesn't cover — `ref` implicitly present via `ComponentProps<'button'>`'s real React 19 typing rather than an explicit member — falls under the existing, already-accepted "curated mode doesn't expand full HTML attributes" limit, not a version-specific gap.

Left the `ReactVersion` fields and CLI flag in place (real, tested, user-facing surface) rather than ripping them out — but didn't invent speculative behavior for a bug that isn't reproducible. If a concrete React-version-dependent extraction difference ever surfaces, this is where it'd get wired in.

### Fixed: bare passthrough identifier alias not renaming the component

**Fixture:** `fixtures/antd/Button.tsx`

Found by a full-fixture validation sweep (`apps/validate`'s comparison across all 20 libraries at once, not a per-fixture check) — `antd/Button/Button` showed `ours found nothing` despite the component clearly being extracted correctly under a different name. antd's real `Button` export:

```ts
const InternalCompoundedButton = React.forwardRef<...>((props, ref) => { ... });
const Button = InternalCompoundedButton;   // (real upstream: `as CompoundedComponent`)
Button.displayName = 'Button';
export default Button;
```

`try_scan_display_name` couldn't help here — it matches the assignment's left-hand object name (`"Button"`) against an existing mapping's name, but the mapping was still `"InternalCompoundedButton"`, so the match never fired. The actual gap was one level earlier: `try_rename_identifier_wrapped_component` (added for Headless UI's `forwardRefWithAs(ButtonFn) as X` — an _unrecognized wrapper call_ around an already-detected component) only matched a `CallExpression` init. antd's shape has no wrapper call at all — just a bare `const NewName = OldName;` passthrough, optionally `as`-cast. Added a second match arm for a bare `Expression::Identifier` init, reusing the same rename logic. `Button` now surfaces with all 23 real props under its actual export name.

## Note: compound components (`Dialog.Trigger`) — investigated, not a real gap

The original worry: `Dialog.Trigger`, `Select.Item` — sub-components hung off a parent via dot access — aren't detected as separate components at all.

Checked every fixture with a multi-component "family" shape. None uses the actual `<Namespace.Member>` dot-access pattern. `fixtures/ariakit`'s `Menu`/`MenuButton`/`MenuItem`/`MenuProvider` are just independently exported, independently named components — nothing to detect beyond what already works. `fixtures/headlessui/Listbox.tsx` gets closest — real Ark UI/Headless UI code does `Object.assign(ListboxRoot, { Button: ListboxButton, ... })` to expose `Listbox.Button` as a deprecated convenience alias — but `ListboxButton` is _also_ independently exported (`export let ListboxButton = forwardRefWithAs(ButtonFn) as _internal_ComponentListboxButton`) as the primary, non-deprecated API, and that already gets detected and extracted correctly today (confirmed: `Listbox`, `ListboxButton`, `ListboxOption` all present in the output, each with accurate props).

What's actually missing, if anything, is purely cosmetic: no dot-qualified display name (`"Listbox.Button"` instead of `"ListboxButton"`) and no parent/child grouping metadata for doc tools that want to nest sub-components under their parent. There's no fixture or test demonstrating this as broken, and no evidence for which of several plausible designs (rename the display name vs. add a separate grouping field) real consumers would actually want — building either now would be speculative, not a fix. Left alone.

## Note: slot recipes — partial support

`defineSlotRecipe` (PandaCSS) and `tv({ slots })` (tailwind-variants) are in the recognized callee list and benefit from the 2026-06-28 arg-index fix. However, slot recipe variant values are objects (`{ root: "class", header: "class" }`) rather than plain strings. The current `try_collect_cva_call` variant value extraction reads only string values, so slot recipe variants will have 0 entries in `global.enums` even after the arg-index fix.

The `RecipeVariantProps<typeof slotRecipe>` reference will fall back to opaque output with `raw: "VariantProps<typeof slotRecipe>"` in the `composes` array. This is documented as a limitation, not a bug.

**To support slot recipes:** extend the variant value extractor to accept both `string` and `ObjectExpression` values, treating the object's string values as the display representation.
