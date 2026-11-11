# Where Caliper differs from react-docgen-typescript and the checker

Recall against RDT (`docs/measurement.md`) is not a measure of correctness; the table under "How the numbers are produced" is the checker's judgment of both tools. Caliper aims for correct types for Storybook and for agents. TypeScript's checker is the arbiter of a prop's type. Agreement with react-docgen-typescript (RDT) is information, not a goal. Where Caliper differs from RDT only because RDT is wrong or less precise, that is intended. Where Caliper is not better, it is a gap, and this page says so.

RDT is a fair reference for a reason that also limits it: it prints the checker's own type text, alias names kept. On most props RDT and the checker are the same string, so "Caliper differs from RDT" almost always means "Caliper differs from the checker", and the question is whether that is acceptable. Caliper is better than RDT in one class only (RDT truncates or picks the wrong declaration). Everywhere else the honest label is "acceptable difference" or "gap".

Each class below has four parts: what differs, why we think Caliper is better (or "not better: a gap" and the fix direction), how the weekly record measures it (and whether the comparator scored it right), and how it is accounted for. The examples were run with Caliper built from `main` at `c5fbb33e` with the weekly record's flags (`--html-attributes full --include-unexported --load-dependencies --collapse-inherited-props none`), RDT from `apps/oracle` with the options of `scripts/rdt.mjs`, and TypeScript 6.0.3 from `apps/oracle` for the checker text. The verdicts are `apps/oracle/src/oracle.ts` run on the same file (the oracle's own checker is `7.1.0-beta`). Caliper's output is the canonical JSON type with empty `description` fields dropped.

## How the numbers are produced

`scripts/weekly.py --oracle` extracts each pinned corpus with the flags above, then `apps/oracle/src/oracle.ts` asks TypeScript 7.1's checker for the type of every own prop of every exported function whose only parameter, or `props` parameter, is the props type. Own means not declared by `@types/react`, and not `ref` or `key`. A prop both sides list is **shared**, and gets one verdict from the structural comparison in `apps/oracle/src/oracle-compare.ts`:

- **identical**: the same type.
- **spelled**: the same type, Caliper naming what the checker expands (or the reverse).
- **less precise**: Caliper's type is looser, unevaluated, or spelled in a way the comparator has not seen.
- **wrong**: Caliper's type describes something else.
- **checker any/error**: the checker has no type (unresolved import, `any`).
- **not judged**: the oracle did not evaluate the checker's type, so nothing was compared (`docs/measurement.md`, `oracle.universe_version` 4). The measurements below were taken before it existed and counted such props as identical.

`agree` is identical plus spelled over the shared props. It is the looser number: `less precise` hides real imprecision and, as the classes below show, some real errors. `wrong` is the number that matters, and the gate: the weekly issue opens above 0.1% of the judged props (`docs/measurement.md`, "The headline"). `wrong` both over-triggers (a comparator artifact, classes 1 to 3) and under-triggers (silent wrong answers scored `less precise`, class 5), so read it with this page.

Measured 2026-11-06 on `main` at `c5fbb33e`, the pinned corpora, `scripts/weekly.py --oracle`:

| Corpus   | Shared props | Less precise | Wrong | Checker any/error |  agree |   wrong | RDT wrong (same props) |
| -------- | -----------: | -----------: | ----: | ----------------: | -----: | ------: | ---------------------: |
| mantine  |       20,104 |        1,828 |     0 |                 8 |  90.9% |       0 |                  0.04% |
| chakra   |      486,832 |           22 |     1 |                 0 | 100.0% | 0.0002% |                 0.006% |
| shadcn   |       13,863 |          738 |     8 |                60 |  94.2% |  0.058% |                  0.59% |
| antd     |        3,426 |          205 |     0 |                 1 |  94.0% |       0 |                  1.05% |
| spectrum |        2,023 |          100 |    22 |               208 |  83.7% |   1.09% |                  5.02% |

The last column is `oracle.tools.rdt.wrong_listed`: the share of the props RDT lists that the checker contradicts. It is over every function the checker takes for a component, as the record was until `universe_version` 2 (2026-11-10); a later record has it over candidate components only (`oracle.tools`) and over every function in `oracle.tools_all_functions`, and its spectrum row has more shared props, because default exports are named (`docs/measurement.md`). react-docgen throws on 51 of 295 antd files and 265 of 1,240 shadcn files (17% and 21%) and so omits their props.

The 31 wrong verdicts are 28 index-signature artifacts (class 1), 2 silent call-signature errors (class 4) and 1 narrower type from a factory (class 6). The 3,201 shared props that are not agree are 2,893 less precise, 31 wrong and 277 checker any/error.

## Classes

### 1. Index signatures (comparator artifact, fixed in the comparator fix)

**What differs.**

```ts
type Layout = { [id: string]: number }
props: { layout?: Layout; slots?: { [slot: string]: boolean } }
```

- RDT: `layout` is `Layout`, `slots` is `{ [slot: string]: boolean; }`.
- Caliper: `{"kind":"object","fields":[{"name":"id","propType":{"kind":"number"},"required":true,"indexKey":{"kind":"string"}}]}` for `layout`, and the same shape with `slot` for `slots`.
- Checker: `Layout | undefined` and `{ [slot: string]: boolean; }`.
- Verdict on main: `wrong` for both props.

**Why.** Caliper is right: the canonical JSON states an index signature as a field carrying `indexKey`, and the type is the one the checker prints. The comparator reads that field as a property named `id` or `slot`, which the checker's type does not have.

**Measured.** 28 of the 31 `wrong` are this, and all 28 are false. After the comparator fix an index signature is compared as an index signature and these props are identical or spelled. Until it merges, spectrum's 22 wrong (1.09%) are above the gate for this reason alone.

**Accounted.** Documented here; the comparator fix removes it.

### 2. `readonly` (comparator artifact, fixed in the comparator fix)

**What differs.**

```ts
props: { tuple?: readonly [number, string] }
```

- RDT: `readonly [number, string]`.
- Caliper: `{"kind":"readonly","element":{"kind":"tuple","elements":[{"type":{"kind":"number"}},{"type":{"kind":"string"}}]}}` (schema version 5 wraps each element to carry its marker and label).
- Checker: `readonly [number, string] | undefined`.
- Verdict on main: `less precise`.

**Why.** Same type, so Caliper is right. The comparator does not know the `readonly` kind and reads it as unevaluated.

**Measured.** 22 props across the corpora score `less precise` for this. After the fix the comparator looks through `readonly`.

**Accounted.** Documented here; the comparator fix removes it. It moves `less precise` into `identical`, so it moves `agree` and not `wrong`.

### 3. `'a' | string` against `string & {}` (comparator artifact, fixed in the comparator fix)

**What differs.**

```ts
type Size = 'sm' | 'md'
props: { size?: Size | (string & {}) }
```

- RDT: `Size | (string & {})`.
- Caliper: `{"kind":"union","members":[{"kind":"stringLiteral","value":"sm"},{"kind":"stringLiteral","value":"md"},{"kind":"string"}]}`.
- Checker: `Size | (string & {}) | undefined`.
- Verdict on main: `less precise`.

**Why.** Caliper writes the pattern's meaning: the literals, then any string. It does not keep the `& {}` trick that stops an editor collapsing the union. For Storybook that is the right control (a select of the literals plus free text) and the type accepts the same values. The comparator scores a `string` beside literals as widening.

**Measured.** About 166 props score `less precise` for this. After the fix the comparator reads `string` beside the checker's `string & {}` as the same type.

**Accounted.** Documented here; the comparator fix removes it.

### 4. Call-signature-only types become an empty object

**What differs.**

```ts
type Fn = { (x: number): string }
type Watch<T> = { (): T; (name: keyof T): T[keyof T] }
props: { f?: Fn; w?: Watch<{ x: number }> }
```

- RDT: `Fn`, `Watch<{ x: number; }>`.
- Caliper: `{"kind":"object","fields":[]}` for both.
- Checker: `Fn | undefined`, `Watch<{ x: number; }> | undefined`.
- Verdict: `wrong` for both. The component has no diagnostics and the props have no `degradedBy`.

**Why not better: a gap.** The extractor turns an object type literal into fields and drops a call signature (`ts_signature_to_object_field` in `crates/core/src/extractor/mod.rs`, the `_ => None` arm), so a type that is only a call signature has no members. Fix direction: read the call signature as a `function` type, and keep it beside the fields when a type has both. A Storybook control and an agent both see "an object with no fields" for a callable prop.

**Measured.** `wrong`, counted: 2 props (react-hook-form's `watch` and `getValues`). The comparator scored it right.

**Accounted.** Unaccounted: silent, no marker, no diagnostic. Since schema version 5 a type literal with overloads, a construct signature, or a signature beside properties is a `callable` with every signature (`Watch<T>` here); before, it was `opaque` (`unsupportedExpression`).

### 5. `Partial`, `Required`, `Omit` and `Pick` inside a type alias lose their effect

**What differs.**

```ts
type Names = 'root' | 'item'
type Base = { x: number; y: string; z?: boolean }
type StylesRecord<N extends string, P> = Partial<Record<N, P>>
type O = Omit<Base, 'x'>
type PK = Pick<Base, 'x'>
props: { inline?: Omit<Base, 'x'>; viaAlias?: O; picked?: PK; styles?: StylesRecord<Names, string> }
```

- RDT: `Omit<Base, "x">`, `O`, `PK`, `Partial<Record<Names, string>>`.
- Caliper, `inline`: `{"kind":"named","name":"Omit","args":[{"kind":"object","fields":[{"name":"x",…},{"name":"y",…},{"name":"z",…}]},{"kind":"stringLiteral","value":"x"}]}`.
- Caliper, `viaAlias` and `picked`: `{"kind":"object","fields":[{"name":"x","propType":{"kind":"number"},"required":true},{"name":"y","propType":{"kind":"string"},"required":true},{"name":"z","propType":{"kind":"boolean"},"required":false}]}`, all three fields, for both.
- Caliper, `styles`: `{"kind":"named","name":"Record","args":[{"kind":"literalUnion","members":["root","item"],"hasDefault":false},{"kind":"string"}]}`. The `Partial` is gone, so every key reads as required.
- Checker: `Omit<Base, "x"> | undefined`, `O | undefined`, `PK | undefined`, `Partial<Record<Names, string>> | undefined`.
- Verdicts: `inline` spelled, `viaAlias` wrong, `picked` wrong, `styles` less precise. No diagnostic, no `degradedBy`.

**Why not better: a gap, and a wrong one.** The same utility written inline in the prop keeps its name and arguments. Reached through an alias, `resolve_type_alias_type` (`crates/core/src/resolver/alias.rs`) forwards `Partial`, `Required`, `Omit` and `Pick` to the base type, so an omitted key reappears, a picked-out key set is the whole base, and a `Partial` loses its optionality. Fix direction: make the alias path return what the inline path already returns (the utility as a `named` type with the base as its argument), or evaluate the key set and the modifier. In Mantine this is the `classNames` and `styles` props of about 230 components, 487 occurrences.

**Measured.** Scored wrongly. The comparator compares an object's field names and not whether a field is optional, so a `Partial` or `Required` that lost its modifier scores `identical`, and Mantine's `Partial<Record<…>>` scores `less precise`. Only a changed key set reaches `wrong`, as `viaAlias` and `picked` do above. In the weekly record the Mantine occurrences are in `less precise`, never in `wrong`, so the gate does not see them.

**Accounted.** Unaccounted: silent.

### 6. Props taken from a name-matched interface while a factory call supplies them

**What differs.** A component whose props come from a factory call such as Chakra's `chakra("div", { variants })`, while an interface of the same name is also declared, gets the interface's props. Chakra's `AbsoluteCenter.axis` is `"horizontal" | "vertical" | "both"` in Caliper and `ConditionalValue<"both" | "horizontal" | "vertical" | undefined>` for the checker (the same applies to `Center.inline` and `InputElement.placement`). This comes from the weekly record's dump, not a reduced file.

**Why not better: a gap.** Caliper is narrower than the real type: the prop accepts responsive values the type does not show. Fix direction: this needs the checker backend (`docs/edge-cases.md`, Detection); the structural resolver cannot know what a factory adds.

**Measured.** `wrong` for `axis` (1 prop in the 31). The comparator scored it right.

**Accounted.** Unaccounted: silent.

### 7. An alias kept by name where the checker expands it, or the reverse

**What differs.**

```ts
type Wrap<T> = { [K in keyof T]?: string }
declare function pick(a: number, b: string): void
props: { mapped?: Wrap<Shape>; record?: Record<'a' | 'b', number>; params?: Parameters<typeof pick>; ret?: ReturnType<() => string> }
```

- RDT: `Wrap<Shape>`, `Record<"a" | "b", number>`, `[a: number, b: string]`, `string`.
- Caliper: `{"kind":"named","name":"Wrap","args":[{"kind":"named","name":"Shape","args":[]}]}`, `{"kind":"named","name":"Record","args":[…]}`, `{"kind":"named","name":"Parameters","args":[{"kind":"named","name":"typeof pick","args":[]}]}`, `{"kind":"named","name":"ReturnType","args":[…]}`.
- Checker: `Wrap<Shape>`, `Record<"a" | "b", number>`, `[a: number, b: string]`, `string`.
- Verdicts: `mapped` spelled, `record` spelled, `params` less precise, `ret` less precise.

**Why.** For an alias the checker also prints by name (`Wrap`, `Record`) the difference is spelling, and a name is the same type, so it is acceptable. The cases where the checker evaluates (`Parameters`, `ReturnType`) are not better: Caliper does not evaluate those two utility types, and RDT shows the value. Fix direction: evaluate the standard utility types whose operands are known. Mantine's `Attributes<PaperFactory>` and `vars` types, left as unexpanded names where the checker expands them to an object or a resolver type (391 occurrences), are the same gap in a larger form.

**Measured.** `spelled` counts toward `agree`. `less precise` mixes two things: a genuine spelling difference, and real imprecision, because the comparator puts any spelling it has not seen under `less precise`. About 1,500 props are an alias or interface kept by name and about 500 a standard utility kept by name; most are spelling, and 487 of the latter are class 5.

**Accounted.** Documented. Unevaluated names also raise a `Cannot resolve type` diagnostic when the name does not resolve; a standard utility left by name does not.

### 8. Caliper is more complete than RDT

**What differs.**

```ts
props: { big?: { field0: number; …80 fields… } | null; many?: …60 object variants… }
```

- RDT: `{ field0: number; field1: string; … 66 more ...; field79: string; } | null` and `{ kind: "variant0"; value0: number; } | … | { ...; } | ... 54 more ...`.
- Caliper: the 80 fields and the 60 variants in full (checked: a union of an 80-field object and `null`, and a union of 60 objects).
- Checker (printed without truncation): the full text.
- Verdict: `identical` for both.

**Why Caliper is better.** RDT prints the checker's type text with its default truncation, so a long type reaches Storybook as `… 54 more ...`, which is not a type. Caliper keeps all of it. The weekly dump also has two cases of RDT picking the wrong declaration (Chakra's `AccordionItem.value` is `string[]` in RDT, `string` in the checker and Caliper) and an incomplete enum (Chakra's `ColorPickerChannelInput.channel`); I did not re-run those here.

**Measured.** RDT's truncated text is unreadable to the oracle's adapter, which scores it `less precise` and never `wrong` (`apps/oracle/src/tool-norm.ts`), so RDT's wrong rate in the table is, if anything, low.

**Accounted.** Nothing to account for in Caliper. The RDT side is the last column of the table above.

### 9. Opaque, with a marker

**What differs.**

```ts
type E = { a: string; b: number }
props: { v?: React.ComponentProps<'input'>['value']; e?: E[keyof E]; tab?: 0 | -1 }
```

- RDT: `string | number | readonly string[]`, `string | number`, `0 | -1`.
- Caliper: `{"kind":"opaque","raw":"React.ComponentProps<\"input\">[\"value\"]","reason":{"type":"indexedAccess",…}}`, `{"kind":"opaque","raw":"E[keyof E]","reason":{"type":"indexedAccess",…}}`, and for `tab` a union of `{"kind":"numberLiteral","value":0.0}` and `{"kind":"opaque","raw":"-1","reason":{"type":"unsupportedExpression"}}`.
- Checker: `string | number | readonly string[] | undefined`, `string | number | undefined`, `0 | -1 | undefined`.
- Verdict: all three `less precise`. Each prop carries `degradedBy` (`indexedAccess`, `indexedAccess`, `unsupportedExpression`), and the run reports an `INDEXED_ACCESS_OPAQUE` or `OPAQUE_TYPE` info diagnostic for each.

**Why not better, but acceptable.** The checker and RDT evaluate these and Caliper does not (an indexed access on a generic, a `-1` literal). The type is flagged as a guess, so a reader or an agent knows. Fix direction: evaluate an indexed access on an interface; the rest needs the checker backend. A negative number literal (`-1`) is a number literal since schema version 5.

**Measured.** `less precise`, counted by the `degradedBy` marker: 547 props in the weekly run. The comparator scored it right. `stats.opaqueCount` is 0 in every corpus and is not a measure.

**Accounted.** Flagged by `degradedBy` and a diagnostic. One exception is not flagged: `PropType::degraded_by` (`crates/core/src/types/output.rs`) skips opaque index keys, so 4 antd props are silently opaque. Unaccounted; fix direction: collect through an object field's `indexKey`.

### 10. `ReactElement`, `JSX.Element` and `ReactChild` become `reactNode`

**What differs.**

```ts
props: { a?: React.ReactElement; b?: JSX.Element; c?: React.ReactNode }
```

- RDT: `ReactElement<unknown, string | JSXElementConstructor<any>>`, `any`, `ReactNode`.
- Caliper: `{"kind":"reactNode"}` for all three.
- Checker: `ReactElement<unknown, string | JSXElementConstructor<any>> | undefined`, `any` (the React 19 types declare no global `JSX`), and the long `ReactNode` union.
- Verdicts: `a` less precise, `b` checker any/error, `c` spelled.

**Why not better, but deliberate.** `reactNode` is wider than `ReactElement`: it also admits strings and numbers. Storybook gives both the same control, and the canonical JSON has one terminal for them (`resolver/react.rs`, `types/output.rs`). Splitting them is a change to the canonical JSON and a schema version bump (ADR 0017). Whether to make that change is open. 236 occurrences.

**Measured.** `less precise` (and `checker any/error` for `JSX.Element`). Scored right.

**Accounted.** Documented here and by the ADR; no marker.

### 11. Intersections left as written

**What differs.**

```ts
type Mode = 'a' | 'b'
type Wide = 'a' | 'b' | 'c'
props: { m?: Mode & Wide }
```

- RDT: `"a" | "b"`.
- Caliper: `{"kind":"intersection","members":[{"kind":"literalUnion","members":["a","b"],…},{"kind":"literalUnion","members":["a","b","c"],…}]}`.
- Checker: `"a" | "b" | undefined`.
- Verdict: `less precise`; no marker.

**Why not better: a gap.** The checker reduces the intersection; Caliper writes both sides. The type is correct but a reader has to do the intersection, and a Storybook control for an intersection of two literal unions is not a select. Fix direction: reduce an intersection of literal unions and of object types when every member is known. 70 props.

**Measured.** `less precise` (reason: intersection member count). Scored right.

**Accounted.** Unaccounted: no marker.

### 12. Type parameters

**What differs.**

```ts
interface GroupProps<Value extends string = string> { defaultValue?: Value | null; options?: Value[] }
export const Group = React.forwardRef<HTMLDivElement, GroupProps>(…) as <Value extends string = string>(
  props: GroupProps<Value> & React.RefAttributes<HTMLDivElement>,
) => React.ReactElement
```

- RDT: `defaultValue` is `string | null` and `options` is `Value[]`, which is inconsistent.
- Caliper: `string | null` and `string[]`.
- Checker: `Value | null | undefined` and `Value[] | undefined`.
- Verdicts: `less precise` for both props of the cast `Group`. For a plain generic function with the same props (`Group2`), Caliper keeps `Value`, RDT prints `string | null` and `Value[]` as well, and both props are `identical`.

**Why.** Replacing a parameter with its default or constraint (33 props) is by design: a Storybook control needs a concrete type, and `string` is what `Value` is until a story says otherwise. It is one instance of the parameter, narrower than the parameter itself, and the checker keeps the parameter, so it is acceptable. The unbound case is a gap: where a parameter has no resolved declaration Caliper prints `unknown` or `any` instead of the constraint (37 props, for example shadcn's `Select.multiple`: Caliper `unknown`, checker `Multiple | undefined`; Mantine's `OverflowList.data`: Caliper `any[]`, checker `T[]`). I could not reduce that case to a small file (a plain generic function, a generic arrow, and a generic over a dependency's interface all keep `T`), so the cause is not pinned down. Fix direction: print the constraint, or the parameter by name.

**Measured.** `less precise` (type parameter instance, or Caliper `any` or `unknown` against a parameter). Scored right.

**Accounted.** The replacement is documented here. The unbound case is unaccounted.

### 13. The checker has no type

**What differs.**

```ts
import { Thing } from 'not-installed-package'
props: { t?: Thing; n?: number }
```

- RDT: `any`.
- Caliper: `{"kind":"named","name":"Thing","args":[]}`, with a `Cannot resolve type 'Thing'` diagnostic.
- Checker: `any`.
- Verdict: `checker any/error`.

**Why Caliper is no worse, and here better.** Where the import does not resolve, the declared name is more information than `any`. But the weekly case is wider: the oracle's tsconfig is one `weekly.py` writes (`program_inputs`) and has no `paths`, so aliased imports do not resolve and the checker has `any`. In the samples Caliper was right (277 props, 208 of them in spectrum).

**Measured.** `checker any/error`: neither agree nor wrong, so it is a blind zone. Giving the oracle the `paths` would convert some of these into judged props and move both `agree` and `wrong`.

**Accounted.** Reported as a count; the cause is documented here.

### 14. Props only the checker finds

**What differs.** Ambient augmentation. A dependency's declaration file adds an attribute to React's types:

```ts
// fake-og/index.d.ts
declare module 'react' { interface HTMLAttributes<T> { tw?: string } }
// box.tsx
import 'fake-og'
export function Box(props: React.ComponentProps<'div'> & { tone?: 'a' | 'b' }) {…}
```

- RDT: `tw` is `string`.
- Caliper: `tw` is not listed (280 props, none of them `tw`).
- Checker: `tw : string | undefined`.

**Why not better: a gap**, and RDT has it. Caliper does not apply a `declare module 'react'` augmentation that arrives through a dependency. Fix direction: apply module augmentation of React from loaded dependencies. In the weekly record this is Next's augmentation in shadcn (55 props, all `tw`), plus 20 in spectrum and 2 in chakra.

**Measured.** `props_missing`, not a verdict, so `agree` and `wrong` never see it. The headline lists it as `missing`.

**Accounted.** Counted, not marked. `docs/STATUS.md` quotes an older figure for shadcn (1,154 of 1,182); the weekly record is the current one.

### 15. Props only Caliper lists

**What differs.** With `--html-attributes full` Caliper lists React's DOM attributes. For `React.ComponentProps<'button'> & { variant?: 'solid' | 'ghost' }` it lists 290 props, RDT lists the same 290, and the oracle's universe is 1 (`variant`), because it excludes `@types/react`.

**Why.** Intentional: `full` is the default so a props table has `onClick`, `aria-*` and `ref`. `curated` lists 1 prop plus 164 in `notableInherited`, and `none` lists 1.

**Measured.** Not measured. The oracle never scores a prop outside its universe, so Caliper-only counts do not appear in any column, and a count of "Caliper-only props" taken from the JSON is inflated by colliding keys and should not be quoted.

**Accounted.** Documented (`docs/rdt-coverage.md`, HtmlAttributeMode).

### 16. RDT's own quirks

What RDT does that Caliper does not, run on small files:

- It drops `children` when the props type is an inline object literal. `function Kids2(props: { children?: React.ReactNode; label?: string })` lists only `label` in RDT; with the same members in a named interface RDT lists `children`. Caliper lists `children` in both, and the checker has it.
- It prints a generic inconsistently: `string | null` for one prop and `Value[]` for the next (class 12).
- It truncates long types (class 8).
- It reports any function with an object-like parameter as a component. For `export function helper(label: string)` RDT lists 50 props, `String`'s methods (`charAt`, `concat`, …), and Caliper reports no component.
- It lists `ref` and `key`, and one component per re-export (`docs/measurement.md`, "The reference").
- It omits whole components: in the weekly record the props of components RDT does not report are 403 (shadcn), 5,957 (chakra), 3,662 (antd) and 2,699 (spectrum) (`missing_by_reason.component`).

**Why Caliper is better.** Each of these is RDT being wrong or incomplete, so it is intended that Caliper differs.

**Measured.** RDT's score against the checker is in the table (`oracle.tools.rdt`), by the same rules as Caliper's. The reference for recall removes the non-components and the repeats (`docs/measurement.md`).

**Accounted.** The reference rules are documented; a recall figure against RDT is information.

### 17. Collapsed inheritance layers

**What differs.** With the default `collapseInheritedProps: dependencies`, a dependency source past the size rule becomes a layer of names. For a props interface extending a 35-prop `SpaceStyleProps` from a package:

- RDT lists `gap3` as `string | number`.
- Caliper, default collapse: 1 prop listed, and `gap3` is a name in the layer's `members` (35 of them), with no type. With `none`: 36 props listed and `gap3` has its type.
- Checker: `gap3 : string | number | undefined`.

**Why not better, at the default.** A CLI, NAPI or agent consumer sees the name and not the type. The Storybook addon runs `none` and lists the type. Fix direction is in `docs/measurement.md`, "Layer-only props".

**Measured.** The weekly record runs with `none`, so `named only` is 0 on every corpus; the 39.6% shadcn figure in `docs/measurement.md` is a local default-collapse run.

**Accounted.** Documented; the layer lists the names.

## What the oracle does not measure

- Whether a prop is required or optional, its default, its description and JSDoc tags, `@deprecated`.
- Nesting beyond `MAX_DEPTH = 3` in `oracle-compare.ts`: below that depth two types compare as identical.
- `@types/react` props, `ref` and `key`: excluded by design.
- Object fields' optionality (class 5): an object is compared by field names.
- Components the checker does not take for a component, and components Caliper reports that are not exported functions.
- Required and default are checked against Storybook's docgen, not the checker (`docs/storybook-parity.md`).

## What each class costs a Storybook user or an agent

- Index signature, `readonly`, `string & {}`: nothing; the type is right, and only the score is wrong.
- Call signature to `{}`: a callable prop shows as an object with no fields.
- Utility inside an alias: the wrong keys, or required keys that are optional.
- Factory-supplied props: a prop's responsive values are missing.
- Alias by name: the reader follows a name; `Parameters` and `ReturnType` show unevaluated.
- Caliper more complete: a long type is whole instead of cut.
- Opaque: the type is flagged as partial.
- `reactNode`: a `ReactElement` prop also reads as accepting text.
- Unreduced intersection: no select control, and an agent must reduce it.
- Type parameters: a concrete type, or `unknown`.
- Checker has no type: nothing; Caliper keeps the declared name.
- Checker-only props: an augmentation's attribute is absent.
- Caliper-only props: a long table.
- RDT quirks: none for Caliper's users.
- Collapsed layers: names without types at the default collapse.
