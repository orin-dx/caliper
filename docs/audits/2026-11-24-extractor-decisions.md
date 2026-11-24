# Audit: name-, peel- and shape-based decisions in the extractor and resolver

Read-only audit of `origin/main` at `git fetch` time (oxc 0.158.0). Scope: `crates/core/src/extractor/**`, `resolver/**`, `import_map.rs`, `known.rs`, `react_types.rs`, `types/ambient.rs`. Line numbers are `origin/main`.

**Reproduced** marks hits run through the CLI: `caliper-docgen extract -s <repro> --json --quiet --no-cache --no-checker --html-attributes none`. The binary was built in worktree `agent-aa40b3efb28981791` (branch `fix-519c`), whose `crates/core/src/extractor`, `resolver/callee.rs` and `resolver/mod.rs` have no diff from `origin/main`. The fixtures are in `/tmp/claude-501/repro/src/{a,b,config,lib}.tsx`. Everything else was found by reading the source; none of it was run under the checker. ADR 0026 makes the checker the type authority, but component _detection_ is still the extractor's, so every wrong component below is published whichever backend runs.

Classes: (a) a decision made by name or spelling, (b) several predicates answering one question, (c) a catch-all arm that makes a decision, (d) peeling wrappers, (e) dependence on oxc node shapes. Severity: **W** = produces a wrong component, prop or type; **M** = only misses one; **S** = style or latent.

## Top 10 by severity

1. **(a) The "renders" evidence is mostly names.** `extractor/evidence.rs` (W, reproduced ×4)
   - :175, a PascalCase callee counts as a render: `return String(x)`, `Immutable.Map(x)`, `Number(x)`.
   - :116-122, `is_children` accepts any `.children`.
   - :21/:26-31, the receiver is ignored for every factory except `h`, so `document.createElement` counts.
   - :17/:96, the bare name `Element` (the DOM global) counts as React's element type when unbound.
   - :112/:219, any `useX(…)` call makes the function render.

   ```tsx
   export const FormatId = (p: IdProps) => String(p.id) // component (typedFunctionExpression)
   export const GetChildren = (n: TreeNode) => n.children // component
   export const MakeDiv = (p: Opts) => document.createElement(p.tag) // component
   export function GetNode(p: Sel): Element {
     return document.querySelector(p.sel)!
   } // component
   ```

   Fix:
   - Drop the PascalCase-call rule, or require the callee to be a binding the file maps as a component (`component_mappings` / `element_helpers`).
   - Accept `children` only as `props.children` or a destructured `children` of the first parameter: `body_facts` already binds the props symbol.
   - Make `is_element_factory_name` require React's or Preact's binding for every name, not only `h`.
   - Remove the bare `Element`: `JSX.Element` is matched through `leftmost_and_rightmost`.
   - Count a hook call only when there is no return-type evidence against it, or drop it.

2. **(a/c) `is_component_wrapper_call` is a list of name conventions.** `extractor/component.rs:934-948` (W, reproduced)
   - `callee.contains("Component")`, `lower.contains("forwardref")`, `with[A-Z]…` and `observer` all count as wrappers.
   - `Expression::CallExpression(_) => return true` accepts every curried call.
   - Through `is_component_alias_call` (:915) this decides alias identity in `alias_shape` (:739), `component_wrapped_by` (uses.rs:84), `record_call` (uses.rs:203) and wrapper evidence (wrapper.rs:109).

   ```tsx
   export function Card(p: CardProps) {
     return <div />
   }
   declare function getComponentName(c: unknown): string
   export const CardName = getComponentName(Card) // published as a component with Card's props
   export const Keyed = keyFor('x')(Card) // curried: also an alias of Card
   ```

   Fix: decide by binding. React's `memo`/`forwardRef` go through `react_api`. Anything else is only a candidate (emit `SkippedCandidate`, or publish only when the checker confirms that the call's return type is a component). At minimum remove `contains("Component")` and the curried `=> true`.

3. **(a) Any function type a factory returns is treated as a component.** `resolver/callee.rs:338-340` (W, reproduced)
   - `CollectedType::Function { params, .. }` is read as a component whose props are `params[0]`, and the return type is never checked.
   - `resolver/mod.rs:1053` ("a call to a function whose result is no component is no component") depends on that check, so nothing stops a non-component.

   ```tsx
   function makeFormatter<T>(): (value: T) => string {
     return (v) => String(v)
   }
   export const Format = makeFormatter<Money>() // component "Format" (factoryResult) with prop `amount`
   ```

   Fix: accept `Function` only when `return_type` is React's element or node type, resolved by binding. Otherwise return `None`.

4. **(b) Sibling paths skip the render evidence.** (W, reproduced ×2)
   - `try_typed_function_expression`, `mapping_from_typed_function` and `visit_function` all call `evidence::renders`.
   - `record_inline_sub_component` (`extractor/interface.rs:488-532`, used by `Object.assign` :443 and `Root.X = fn` :483) does not.
   - Neither does `try_anonymous_default_export_component` (`component.rs:557-598`).

   ```tsx
   function Api() {
     return null
   }
   export const Client = Object.assign(Api, { Format: (v: ApiProps) => v.url.toUpperCase() }) // "Format" component
   // config.tsx
   export default function (opts: Options) {
     return { ...opts, ready: true }
   } // "Config" component
   ```

   Fix: call `renders(&FunctionShape::…)` in both, and record the reason as a skip. Long term, see M2.

5. **(a) A synthesized props name can collide with one the source declares.** `extractor/component.rs:864-877` (W, reproduced)
   - `push_imported_alias` names the props type `"ComponentProps"`, which resolves through normal scoped lookup.
   - A file that declares its own `ComponentProps` gives every imported-member alias those props.

   ```tsx
   import * as Lib from './lib' // Lib.Thing has props { real: number }
   type ComponentProps = { local: boolean }
   export const Alias = Lib.Thing // output: Alias { local: boolean }
   ```

   Fix: use a name no source can write, as `record_declared_value_type` already does with `"typeof X"`. Better, make it a `TypeName::Synthesized` variant (M6).

6. **(d) An `as` assertion is read as `as const`, or as transparent, for literal types.** (W, reproduced ×2)
   - `extractor/interface.rs:19-27` (`as_const_literal`) accepts any `TSAsExpression` or `TSTypeAssertion` as `as const`. It feeds `try_collect_const_enum` and `try_collect_const_array`.
   - `extractor/values.rs:16-28` (`peel`) treats `as T` and `<T>x` as transparent ("none of them changes which literal it is"). That is true of the value, but the type is what gets recorded.

   ```ts
   export const SIZES = ['sm', 'md'] as string[] // (typeof SIZES)[number] → "sm" | "md"  (should be string)
   export const DEFAULT_SIZE = 'sm' as string // typeof DEFAULT_SIZE    → "sm"        (should be string)
   ```

   Fix: recognize `as const` explicitly (a `TSTypeReference` whose name is `const`). For any other `as T` or `<T>x`, record the asserted type, or nothing. `satisfies`, `!` and parentheses stay transparent.

7. **(a/b) `Omit`/`Pick`/`Partial`/`Required`/`Readonly` are classified by spelling and never re-checked.** (W, reproduced)
   - `extractor/alias.rs:18-160` classifies these into `CollectedTypeAlias::{Omit,Pick,Partial,Required}` by spelling.
   - `resolver/alias.rs:38ff` applies them without `may_be_ts_lib_type`.
   - The `extends` path does check the binding (`chain.rs:201,232`, `alias.rs:291`), so the two sibling paths disagree.

   ```ts
   type Partial<T> = { [K in keyof T]?: T[K] | null };   // the project's own
   type NullableBase = Partial<Base>;
   export function Nullable(p: NullableBase) { return <div/>; }   // output: a: string (optional), `| null` lost
   ```

   Fix: in the resolver, use the classified variant only when `may_be_ts_lib_type(name)` holds. Otherwise resolve as `Passthrough` of the written `Named`. Simpler still: the extractor records only `Named` and the resolver classifies after binding (ADR 0025, resolver owns scoping).

8. **(d) Finding 3's class is still open at several callee and export sites.** (W, by reading) `peel_expr` (`component.rs:20-31`) treats `as` as transparent, and these sites use it on a callee or export:
   - `try_record_factory_call` (`wrapper.rs:491`) takes the callee through a cast: `(makeX as unknown as OtherFactory)<P>()` uses `makeX`'s recorded result.
   - `react_api` (`mod.rs:735,744`): `(memo as unknown as Custom)(X)` is read as React's memo.
   - `is_forward_ref_call` (:886) and `is_component_wrapper_call` (:936).
   - `pending_function_aliases` (`visit.rs:355-358`) copies `f`'s result to `const g = f as G`.
   - The default export (`visit.rs:660`, `:714-720` via `component_wrapped_by`, `reexports.rs:83`): `export default Button as unknown as FC<Other>` exports `Button` with Button's props. The assertion is dropped with no diagnostic.
   - Only `asserted.rs` handles this, for one shape: a default export asserted to a function type, with a single `as`.

   Fix: one peel that returns `Asserted(type)` (M8). Callee consumers decline or use the asserted type. Export consumers apply the asserted type the way `asserted.rs` does, for every shape.

9. **(a) An unbound name is taken as React's, and local declarations are ignored.** (W, by reading)
   - `react_api` (`extractor/mod.rs:733-759`), `react_type_export` (:765-785) and `may_be_react` (:787-793) look only at imports.
   - So `const memo = (f) => f`, `function forwardRef(…)` or `type FC<P> = (p: P) => string` declared in the file all count as React's.
   - `resolver/callee.rs:22-28,342-363` (`is_component_type`) strips `React.` with no binding check at all, and checks before `lookup_interface`, so a local `interface ExoticComponent<T>` is not found first.
   - `react_types.rs:306-372` lists DOM globals (`MouseEvent`, `KeyboardEvent`, `FocusEvent`, …) as React builtins, and `may_be_react_builtin` gives them React's meaning when unbound.

   ```tsx
   type FC<P> = (p: P) => string // the project's own
   export const Label: FC<User> = (u) => u.name // component (FcAnnotation)
   ```

   Fix: one binding oracle (M3) that sees local declarations, as `resolver::name_binding` already does for types. Add `may_be_react_builtin` to `callee.rs:342,346,363`.

10. **(b) Finding 2's class is still open.** (W/M, by reading)
    - `interface.rs:408-415` (`scan_object_assign_call`) is a third copy of "is this `Object.assign`?" beside `component.rs:925` and `uses.rs:119`. Its first argument is unpeeled (`Argument::Identifier`, :416), while `aliased_component` (`component.rs:921`) peels.
    - "Which argument is the component" has three answers:
      - the first: `aliased_component`, `component_argument_evidence` (wrapper.rs:91), `mapping_from_typed_function`;
      - the first argument of _any_ call: `body_facts::unwrap` (:141-157);
      - the last: `factory_props.rs:37`.
    ```tsx
    export const Root = Object.assign(RootImpl as typeof RootImpl, { Item: (p: ItemProps) => <li /> }) // alias yes, inline Item scan no
    export const W = withOptions(
      (p: A) => <a />,
      (q: B) => <b />,
    ) // detection reads A, factory_props attaches B
    ```
    Fix: `scan_object_assign_call` calls `is_object_assign_call` and `aliased_component`. A single `component_argument(call) -> Option<&Expression>` is used by detection, body facts and `factory_props`.

## All hits by class

### (a) Decisions by name or spelling — 24

| # | file:line | decides | counter-example | sev | binding/provenance alternative |
| --- | --- | --- | --- | --- | --- |
| a1 | extractor/evidence.rs:175 | a call renders when its callee is PascalCase | `(p: P) => String(p.id)` | W | callee bound to a mapped component or `element_helpers` |
| a2 | evidence.rs:116-122 | `x.children` renders | `(n: TreeNode) => n.children` | W | props symbol (body_facts' `oxc_semantic`) |
| a3 | evidence.rs:21,26-31,34-46 | element factory by name; receiver ignored except for `h` | `document.createElement(t)` | W | import binding of `createElement`, `jsx`, … |
| a4 | evidence.rs:17,96 | `Element` and `ReactNode` return types are React's when unbound | `(): Element => document.body` | W | React import or global `JSX` only; DOM `Element` excluded |
| a5 | evidence.rs:112,219 | any `useX()` call renders | `function Compute(p: P) { return useMemo(() => p.x, []) }` | W | hook proves nothing about the return value; drop it or require React's hook binding |
| a6 | extractor/component.rs:934-948 | wrapper by name; curried calls always | `getComponentName(Card)`, `keyFor(x)(Card)` | W | `react_api` + checker confirmation |
| a7 | component.rs:882-892 | any `forwardRef` not imported from React is a forwardRef | a local `function forwardRef(x){}` | W (rule=ForwardRef) | import specifier allow-list (React Aria) |
| a8 | component.rs:925-929 | `Object` by spelling | `import { Object } from "./model"` | M | unbound global check |
| a9 | component.rs:820-862 | `Imported.Pascal` / `{ Pascal } = Imported` is a component | `import * as tokens…; export const Primary = tokens.Primary` | W (unless the resolver drops it) | the checker or the target's declaration |
| a10 | extractor/mod.rs:733-793 | unbound means React's (`memo`, `forwardRef`, `FC`, …) | local `type FC<P>` / `const memo` | W | local declarations plus imports |
| a11 | visit.rs:365-371, 477, 612; component.rs:126-133 | a component candidate must be PascalCase | `export const button = forwardRef<…, P>(…)` | M | declaration + export (ADR 0027) |
| a12 | extractor/alias.rs:18-160 | `Omit`/`Pick`/`Partial`/`Required`/`Readonly` by spelling | own `type Partial<T>` (reproduced) | W | `may_be_ts_lib_type` |
| a13 | extractor/mod.rs:797-830 | `classify_extends` sets `Builtin` by spelling before the import check | `interface P extends HTMLAttributes` from `./mytypes` | S (resolver re-checks at extends.rs:27-37: two owners) | let the resolver decide |
| a14 | wrapper.rs:49-52, 65-68, 555 | `Element`/`HTML*Element` are never props; a PascalCase argument is a component; `*Props` from another file are props | `withX<HTMLDivElement, ThemeProps>(InitialState)` | W | declaration lookup |
| a15 | resolver/callee.rs:22-28, 342-363 | `FC`/`ExoticComponent`/… by spelling, before the local lookup | local `interface ExoticComponent<T>` | W | `may_be_react_builtin` |
| a16 | known.rs:79-114 (via precedence.rs:117, chain.rs:36) | an unresolved `RenderProps`, `SlotProps`, `VariantProps`, `HTMLChakraProps` gets a library's meaning by spelling; a `Props` result emits no diagnostic | `import { RenderProps } from "@/types"` with an unconfigured path alias | W | specifier allow-list per pattern + `builtin_fallback_diagnostic` |
| a17 | resolver/named.rs:238-272 | `*HTMLAttributes`, `HTML*Element`, `SVG*Element`, `Object`… returned silently as `Named` | an unresolvable project `FooHTMLAttributes` | S (no diagnostic) | diagnostic when bound but unresolved |
| a18 | types/ambient.rs:96-120 | `*HTMLAttributes`, `*StyleProps`, `SystemProps` sources are ambient by name | the project's own `interface SpacingStyleProps` with 40 props, collapsed as a layer | W (props hidden) | `origin::declaration_origin(file)` only |
| a19 | react_types.rs:306-372 | DOM globals (`MouseEvent` …) are React builtins | `onPick?: (e: MouseEvent) => void` with no import | S/W | unbound and a TS lib global means TS lib |
| a20 | resolver/html.rs:376-390 | attribute type by name, `_ => String` | `onMouseEnter` becomes `string` | W | `Unknown` + diagnostic |
| a21 | resolver/mod.rs:1002-1021 | `aliases_project_component` pairs by display name (ADR 0027) | `const { Option } = Select` with an unrelated project `Option` | M | `ComponentId` / exports |
| a22 | extractor/mod.rs:459-465 | a static member's type ends with `Component` | `Item: FC<P>` is not indexed | M | React binding |
| a23 | interface.rs:468 | `memo`/`forwardRef` by spelling for `Root.X = call` | `Root.X = React.memo(…)` from a non-React `React` | S | `react_api` |
| a24 | component.rs:345-350 | inner named function must be PascalCase | `memo(function button(p: P){…})` | M | evidence, not case |

### (b) Several predicates for one question — 11

| # | question | sites | should be |
| --- | --- | --- | --- |
| b1 | is this `Object.assign`? | component.rs:925, interface.rs:408-415 (copy, unpeeled args), uses.rs:119 | `is_object_assign_call` + `aliased_component` only |
| b2 | is this forwardRef/memo? | mod.rs:733 `react_api`, component.rs:882 `is_forward_ref_call`, component.rs:943-947, interface.rs:468, mod.rs:1547 `is_react_wrapper` | `Callee::React(ReactApi)` from one classifier |
| b3 | is this name React's? | mod.rs:733/765/787; component.rs:393-404 `is_react_type` (accepts bare names and `React.` with no import check); component.rs:499-513 `is_element`; evidence.rs:369 `Foreign`; resolver/mod.rs:148-167 | one `ReactBinding` oracle shared by extractor and resolver |
| b4 | does this type mean "renders"? | evidence.rs:85 `type_renders` / :388 `names_element` vs component.rs:499-520 (`ReactNode` accepted by the first, not the second; unbound `ReactElement` likewise) | one `is_element_type(ty, binding)` |
| b5 | which React types carry props, and at which argument? | component.rs:75 (`ForwardRefComponent`, a Fluent name, inside a React-binding gate), :110, :405-413; resolver/callee.rs:22; evaluate.rs:81; mod.rs:462 | `enum ReactComponentType { Fc, ForwardRefExotic, MemoExotic, Exotic, … }` with `props_arg()` |
| b6 | which argument is the component? | component.rs:920 (first), wrapper.rs:91 (first), body_facts.rs:141-157 (first of any call), factory_props.rs:37 (last) | `component_argument(call)` |
| b7 | does an assertion replace the props? | asserted.rs:24 (default export, function type, single `as`); component.rs:100 (const, FC/FRE type ref, `as` only); wrapper.rs:674 (`as` + `<T>`); wrapper.rs:691 (`as` only); visit.rs:660/714 peel it away | `Peel::Asserted(ty)` handled at one place |
| b8 | what does this module export? | visit.rs:122 `declaration_name` (first declarator only) vs :130-157 (all declarators); reexports.rs:79-91 copies visit.rs:657-664 (a parity test exists) | one `exports_of(statement)`; `export const A = …, B = …` currently loses `B` from `LexedExport` |
| b9 | what is the callee's name? | component.rs:895 `extract_callee_name` (unpeeled) vs wrapper.rs:491 (peeled) vs evidence.rs:104 | the `Callee` classifier |
| b10 | is `Omit`/`Partial` TypeScript's? | resolver chain.rs:201/232, alias.rs:291 check the binding; resolver alias.rs:38 on extractor-classified aliases does not | the resolver classifies after the binding check |
| b11 | must a candidate function show it renders? | yes: component.rs:262, :355, visit.rs:480/488; no: interface.rs:488, component.rs:557, callee.rs:338 | a constructor that demands evidence (M2) |

### (c) Catch-all arms that decide — 9

| # | file:line | match over | silently does |
| --- | --- | --- | --- |
| c1 | component.rs:939-940 | callee `Expression` | `CallExpression(_) => true` accepts every curried call as a wrapper; `_ => false` rejects e.g. a `ChainExpression` callee (`a?.b(X)`) |
| c2 | evidence.rs:177 | returned `Expression` | `e => value_has_element(e)`; together with the name arms above it decides "renders" |
| c3 | evidence.rs:44, :100, :398 | callee kind, `TSType` | a computed member callee (`R["createElement"]`) or new `TSType` variant answers "does not render" (M, but silent) |
| c4 | extractor/mod.rs:1317 | `Expression` | `_ => "unknown"` is a sentinel string that then flows into `react_type_export("unknown")` (heritage `extends (Base as any)`) and `static_member_assignment` |
| c5 | extractor/mod.rs:1514 | `Declaration` | `_ => None` hides `TSImportEqualsDeclaration` (`export import X = NS.X` is not exported) and any new variant; the exhaustive twin is written.rs:551-567 |
| c6 | extractor/alias.rs:216 | `TSType` | any new or unhandled type is a `Passthrough`; correct today by design, silent tomorrow |
| c7 | interface.rs:22-26 | initializer `Expression` | treats every `as T` as `as const` (hit 6) |
| c8 | component.rs:28 / values.rs:25 | wrapper `Expression` | `_ => return expr`; `peel_expr` misses `TSTypeAssertion` and `TSInstantiationExpression`, `values::peel` misses `TSInstantiationExpression` |
| c9 | resolver/html.rs:390 | attribute name | `_ => PropType::String` invents a type (hit a20) |

Plain walks with a catch-all were not counted (`defaults.rs`, `enum_fold.rs`, `factory_props.rs:147`, `written.rs:418`). `body_facts.rs:543` is a model: `_ => None` there means "handed on", which is the conservative answer.

### (d) Peeling — 10

| # | file:line | wrappers | correct? |
| --- | --- | --- | --- |
| d1 | values.rs:17-28 | parens, `as`, `satisfies`, `<T>`, `!` | **No**: `as T` and `<T>` replace the type (hit 6) |
| d2 | interface.rs:19-27 | `as`, `<T>` | **No**: reads any assertion as `as const` (hit 6); misses `[…] as const satisfies X` (outer node is a `satisfies`) |
| d3 | component.rs:20-31 `peel_expr` | parens, `as`, `satisfies`, `!` | Right for _values_ (alias identity); **wrong for callees and exports** (hit 8): mod.rs:735,744, component.rs:886,936, wrapper.rs:491, uses.rs:200, visit.rs:356,660, reexports.rs:83 |
| d4 | visit.rs:355-358 | via d3 | **No**: `const g = f as G` gets `f`'s function result |
| d5 | uses.rs:82-92 `component_wrapped_by` | via d3 | **No**: `export default forwardRef(Inner) as T` exports Inner, and T is dropped without a diagnostic |
| d6 | wrapper.rs:649 vs :711 | `returned_local_call` peels `as`/`<T>`/`satisfies`; `returned_local_asserted` reads only `as` | **No** in `.ts`: `const C = <Other>make<P>(); return C` uses `make`'s result |
| d7 | asserted.rs:26-30 | parens only, then one `as` | M: `X as unknown as (p: P) => E` and `<…>X` are missed |
| d8 | interface.rs:56-73 `is_class_value` | parens, `as`, `satisfies` | inconsistent with the other peelers (no `!` or `<T>`); harmless |
| d9 | body_facts.rs:458-473 | all six expression wrappers | Correct (it reads keys, not types) and the only complete list |
| d10 | Types: mod.rs:1088, 1266; component.rs:192, 673; evidence.rs:99, 397; alias.rs:195; asserted.rs:15; written.rs:391, 455, 535, 594 | `TSParenthesizedType` | dead under `preserve_parens: false` (e1). `readonly` (`TSTypeOperator`) is handled structurally and exhaustively (mod.rs:1100-1107, written.rs:474-480), which is correct |

Unpeeled reads (the family of finding 6):

- interface.rs:409/:416, the callee and first argument of `Object.assign`;
- interface.rs:452, the parent of a member assignment;
- component.rs:652, a class expression;
- evidence.rs:34-46, the callee of an element factory;
- uses.rs:205, the rendered argument;
- component.rs:895, `extract_callee_name`;
- wrapper.rs:372, `wrapper_callee`.

### (e) Dependence on oxc node shapes — 8

| # | site | how a bump would show |
| --- | --- | --- |
| e1 | `preserve_parens: false` at extractor/mod.rs:192, reexports.rs:24, written.rs:104 | oxc 0.158 drops both `ParenthesizedExpression` and `TSParenthesizedType`, so about 20 paren arms are dead. A new parse site with default options (or a default change) silently breaks every matcher that has no paren arm (b1, c1, d7, the unpeeled reads above). **Silent.** |
| e2 | the expression-wrapper list, written out in 7 places (component.rs:24-27, values.rs:20-24, interface.rs:22-24, 58-60, enum_fold.rs:103-107, body_facts.rs:462-467, asserted.rs:26) | a new wrapper variant compiles and falls into `_ =>` everywhere. **Silent** (this is finding 5) |
| e3 | `Declaration`: mod.rs:1501-1516 (`_ =>`) vs written.rs:551-567 (exhaustive) | the split into `TSNamespaceDeclaration`/`TSExternalModuleDeclaration`/`TSGlobalDeclaration` broke the compile of written.rs only; mod.rs is **silent** |
| e4 | `TSType`: written.rs:443-502 is exhaustive (compile error); evidence.rs:100, mod.rs:1299, alias.rs:216, component.rs:200, asserted.rs:17 use `_ =>` | **Silent** at the latter |
| e5 | `TSTypeOperatorOperator` matches (mod.rs:1100, written.rs:474) | exhaustive, so a **compile error** |
| e6 | `class.heritage.{expression,type_arguments}` (component.rs:604,621; visit.rs:617) | a rename is a **compile error**; a new heritage shape (`extends (X)<T>` as an instantiation expression) is **silent** in `expression_to_ident_name` (c4) |
| e7 | `.without_parentheses()` (10 call sites) | a no-op today; if oxc widens it toward `get_inner_expression` (TS wrappers too), assertions silently become transparent at those sites |
| e8 | `JSXElementName::IdentifierReference` only (uses.rs:211) | a member tag `<UI.Button>` is not counted as a render; a new JSX name kind is **silent** |

**Counts:** (a) 24, (b) 11, (c) 9, (d) 10, (e) 8. Of these, 12 are reproduced or confirmed wrong output (W) in the top 10, and 9 more are W by reading.

## Model and architecture changes

Ordered by what they prevent per unit of cost. "Findings" refers to the six recent findings.

1. **M1. One callee and value classifier on bindings: `extractor/decide.rs`.** Highest priority.
   - Build `oxc_semantic::Scoping` once in `parse_file`; `body_facts::attach` already builds it, so this is a hoist.
   - Every detector receives a sealed classification, never a raw `&Expression`:
     ```rust
     enum Callee { React(ReactApi), ObjectAssign, ElementFactory, Local(SymbolId), Imported { specifier, export }, Asserted(&TSType), Unknown }
     enum ValueRef { Component(Subject), Imported{..}, Asserted { inner, ty }, Other }
     ```
   - `Callee` comes from the symbol: an import's specifier and export, or a local declaration. Spelling never decides it. An unbound name is a global only when no declaration exists.
   - Cost: medium-high, 2-3 PRs. Measure the memory and time of building semantics for every `.tsx` (the memory "measure memory and time" applies).
   - Prevents findings 1, 2, 3 and 6, and hits 2, 8, 9 and 10.

2. **M8, inside M1. A typed peel.**

   ```rust
   enum Peeled<'b,'a> { Transparent(&'b Expression<'a>) /* parens, satisfies, ! */, Instantiated { expr, args }, Asserted { expr, ty: AssertedType } }
   enum AssertedType { Const, Type(&TSType) }
   ```
   - Exactly one function matches the wrapper variants, and it is exhaustive over `Expression` with no `_ =>`. A new oxc wrapper is a compile error in that one place.
   - Callers must handle `Asserted`; they cannot get a transparent answer by accident.
   - Cost: low once M1 exists.
   - Prevents findings 3, 5 and 6, and hits 6 and 8.

3. **M2. Evidence-carrying construction.**
   - `ComponentMapping::new` becomes private.
   - The public constructors require a `ComponentEvidence` value, which only `evidence::renders`, a React component type annotation, a class `extends` React's `Component`, or a checker confirmation can produce. Today's `DetectionRule` becomes a field of that evidence.
   - A sibling path cannot skip evidence because it cannot construct a mapping without it.
   - Cost: low-medium, one PR, mostly mechanical.
   - Prevents finding 1's family and hits 3 and 4 (b11).

4. **M4 + M5. One predicate per decision, enforced by an architecture test.**
   - `decide.rs` holds the only implementations of: is-object-assign, react-api, component-argument, renders, is-element-type, React component type and its props argument, and the export of a statement.
   - A Rust test (no new script) reads `crates/core/src/{extractor,resolver}/**/*.rs` and fails when any of these appear outside `decide.rs` and a small allow-list:
     - the string literals `"assign"`, `"forwardRef"`, `"memo"`, `"React."`, `"Component"`, `"Props"`;
     - `is_pascal_case(`;
     - `strip_prefix("React.")`;
     - `_ =>` / `other =>` inside a `match` whose scrutinee is typed `Expression`, `TSType`, `Declaration` or `Statement`.
   - Alternatively, add `#![deny(clippy::wildcard_enum_match_arm)]` on the decision modules (evidence.rs, component.rs, wrapper.rs, decide.rs).
   - Cost: low.
   - Prevents findings 2 and 4 directly, and finding 5 through the exhaustive peel.

5. **M3. One React-binding oracle for the extractor and the resolver.**
   - Store React references resolved once at extraction, as `CollectedType::React(ReactExport)` or a flag on `Named`, so no reader re-derives meaning from the `React.` spelling (about 20 `strip_prefix("React.")` sites).
   - An unbound name that is also a TS lib global (`Element`, `MouseEvent`) is TS lib, not React.
   - Cost: medium (cache schema bump per ADR 0003).
   - Prevents hits 1 (a4), 9, a13, a15, a19 and b3/b5.

6. **M6. Synthesized names a source cannot write.**
   - A `TypeName::Synthesized(SynthKind, u32)` variant, or a NUL-prefixed key, for `ComponentProps` aliases, `__XCastProps`, `synthesized_alias_name` and `typeof X`.
   - Cost: low.
   - Prevents hit 5.

7. **M7. One parse entry point.**
   - `parse_program(path, source)` owns `ParseOptions`; the three sites call it.
   - Add a test asserting that no `ParenthesizedExpression` or `TSParenthesizedType` node is produced, then delete the dead paren arms. They hide which matchers would break.
   - Cost: low.
   - Prevents finding 5's silent variant.

8. **M9. ADR amendment (0027 or a new ADR beside 0025 and 0028): "a name is a candidate filter, never a verdict".**
   - A spelling may _widen_ the candidate set (PascalCase, `with*`, `*Props`), and every candidate a spelling admitted carries a `Heuristic` reason.
   - It is published only with evidence from M2 or a checker confirmation (ADR 0026: the checker can say whether the export's type is callable and returns a React node).
   - A spelling may never _narrow_ a binding's meaning (`Partial`, `Element`, `ComponentProps`) without the binding check.
   - The only exempt spelling rule is the JSX lowercase intrinsic rule, which is the language's own.
   - Cost: an ADR plus a checker confirmation request (medium).
   - Prevents finding 1 and hits 1-4 whenever the checker is on.

9. **M10. A counter-example suite.**
   - Turn every TypeScript snippet in this report into one row of an extractor test asserting _exact_ output: not detected, or the exact props and types (AGENTS.md: assert content).
   - Each row fails on base today.
   - Cost: low, and it is the evidence the fixes above need.

**Suggested order:** M10 (pins the behaviour), then M2 and M6 (small, close hits 3-5), then M4/M5 (stops recurrence), then M1+M8 and M3 (structural), with M9 written alongside M1.
