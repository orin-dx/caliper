# Deterministic construct suite

One small fixture per type construct, each compared with what TypeScript's checker says. Unlike the weekly oracle (`scripts/weekly.py --oracle`), it runs on a handful of files in seconds, needs no corpus, and fails on any change in how Caliper reads a construct.

## Layout

```
construct-suite/
  tsconfig.json              the options the checker reads the fixtures with
  expectations.json          row id -> allowed verdicts (and the reason for each gap)
  <family>/<NN>-<slug>.tsx   a fixture
  <family>/<NN>-<slug>.golden.json   its committed golden, written by construct:capture
  <family>/<NN>-<slug>.<Component>.golden.json   the golden of each component of a fixture that lists several
```

A row id is `<family>/<NN>-<slug>`, or `<family>/<NN>-<slug>#<Component>` for a component of a fixture that lists several. The harness is `../src/construct-suite.ts` (pure helpers), `../src/construct-capture.ts` (the capture script) and `../tests/construct-suite.test.ts`. `seed/` holds the three rows that prove the harness. The other families are `literals`, `objects`, `tuples-arrays`, `utility-mapped`, `generics`, `react` and `exports`.

A fixture declares one `interface Props` with one prop `p`, and exports a component `Row` that takes it:

```tsx
interface Props {
  p: 'small' | 'large'
}

export function Row(props: Props) {
  return null
}
```

A fixture of several components (the `exports` family: which component is which export, ADR 0027) lists them in a comment, each as `<display name>=<export path>`, and gives each its own `interface` with a prop `p` of a type no other component in the file has, so a component answered with another's props is `wrong`. The export path is the export name (`default` for the default export), then the property of it an attached sub-component is reached through:

```tsx
// components: Item=default.Item Menu=default
interface ItemProps {
  p: 'item'
}
interface MenuProps {
  p: 'menu'
}
function Item(props: ItemProps) {
  return null
}
function Menu(props: MenuProps) {
  return null
}
Menu.Item = Item
export default Menu
```

The capture finds each component by its export path, and the tests find Caliper's by file and display name.

## Commands

```sh
# Write the golden files: the checker (typescript-beta, the independent arbiter) over all fixtures in one program.
pnpm --filter caliper-docgen-oracle construct:capture

# Re-capture and fail on any difference from the committed goldens. Run it when the TypeScript pin changes.
pnpm --filter caliper-docgen-oracle construct:check      # same as: construct:capture --check

# The suite itself, with the package tests.
pnpm --filter caliper-docgen-oracle test
```

The test runs one `extract` over this directory with the weekly's canonical options (`--html-attributes full --include-unexported --load-dependencies --collapse-inherited-props none`, plus `--no-cache --quiet`) and compares prop `p` of each row's component with its golden. The binary is `$CALIPER_BIN`, or `target/release/caliper-docgen`. Without one the suite skips with a message and the rest of the package tests still pass.

## Facts

Besides the type, a golden holds the `facts` both lanes are held to on every row (`Facts` and `factDifferences` in `../src/construct-suite.ts`), each captured from the Go checker and its AST:

- `description` (`getDocumentationComment`) and `tags` (`getJsDocTags`, by name, the last of a repeated tag winning, text trimmed);
- `default`: the text of `@default` or `@defaultValue`, else null (a fixture writes no code default);
- `required`: the property is not optional;
- `parent`: react-docgen-typescript's, the interface the prop's first declaration is a member of, by name and by file relative to this directory; null for a member of a type literal;
- `order`: the members of the union the declaration writes (directly, or through a non-generic alias it names), in written order; null when it writes none. Caliper's union keeps the order when the members it states for them stand in that order.

A fact Caliper states differently fails the row unless its expectation lists it under `"facts"` with the reason (`{ "verdicts": [...], "facts": { "order": "<why>" } }`), and a listed fact that agrees fails as stale. No row lists one today. The checker lane may not list a fact the structural expectation does not: the checker supplies what the structural path states (ADR 0026, S7a).

## Adding a row

1. Write `<family>/<NN>-<slug>.tsx` (two digits, lowercase slug).
2. `pnpm --filter caliper-docgen-oracle construct:capture`, and read the new `.golden.json`: `text` must be the type you meant.
3. Run the test. It names the verdict of any row without an expectation as a missing entry.
4. Add the row to `expectations.json`: `{ "verdicts": ["identical", "spelled"] }`, or, if Caliper falls short, `{ "verdicts": ["<what it is today>"], "gap": "<why>" }`. Commit the fixture, golden and expectation together.

## What a verdict means

Verdicts come from `compare` in `../src/oracle-compare.ts`, the weekly oracle's own comparison.

- `identical`, `spelled`: green. Caliper and the checker state the same type (`spelled` is the same type written another way, such as `'a' | string`).
- `less precise`, `wrong`, `missing`: Caliper's type is looser, describes something else, or the prop is not listed.
- `opaque with marker`: less precise, and Caliper said so (`degradedBy` on the prop). A marked type is honest; an unmarked `less precise` is a silent loss. A `wrong` type stays `wrong` with a marker.
- `checker any/error`: the golden itself has no type. That is a broken fixture, not a result.
- `not judged`: the golden's type was cut or is a kind the checker normalizer does not read, so nothing was compared. A broken fixture too; it used to read `identical`.

The row verdicts and their order are `ROW_VERDICTS` in `../src/vocabulary.ts`.

A row is green or a documented gap. A gap lists the verdicts it may have, none of them green, and a `gap` reason. The test fails when a row moves either way: worse is a regression; better means the gap closed, so update the expectation (to green, dropping `gap`). A gap never allows `identical` or `spelled`, so an improvement cannot pass unnoticed.

## CI

The suite needs Caliper's release binary. A CI job should build it before the package tests and point the test at it:

```sh
cargo build --release -p caliper-docgen
CALIPER_BIN="$PWD/target/release/caliper-docgen" pnpm --filter caliper-docgen-oracle test
pnpm --filter caliper-docgen-oracle construct:check
```

Without the build step the suite skips, which is green but covers nothing; a job that must run it should check that the binary exists first.

## tsrs parity

ADR 0026 makes the Go checker (`typescript-beta`) the type authority and lets tsrs, the Rust port, stand in for it only while it agrees. `construct:parity` is that agreement check: it runs the same one-program capture as `construct:capture` once with the Go checker and once with the exact `tsrs` version pinned in `apps/oracle/package.json`, normalizes both through `src/oracle-checker.ts`, and compares every row on four facts: the normalized type, the `typeToString` text, and the prop type's `flags` and `intrinsicName` (the facts Caliper reads through the API to tell the error type from `any`). It needs no corpora and no network, and takes a couple of seconds.

```sh
pnpm --filter caliper-docgen-oracle construct:parity   # or: just test-parity
```

Run it on every tsrs bump, before changing the pin in the version allow-lists (`crates/tsc-client/src/supervisor.rs`, `crates/semantic/src/lib.rs`), together with the construct suite. Bump the `tsrs` devDependency of `apps/oracle` first, run it, and change the Rust pins only when it passes. The weekly oracle keeps the Go checker as the independent arbiter; this run does not replace it.

A reported difference is a row where tsrs gives a different answer than the Go checker. The report names the row, both `typeToString` texts, the differing facts, and a diff of the two normalized forms (`-` Go, `+` tsrs). Treat it as a tsrs bug or a protocol change until shown otherwise: do not pin that tsrs version, and report it upstream.

`parity-allowlist.json` maps a row id to the reason its difference is harmless (for example a lib file path inside an `import("...")` text). Add an entry only for a difference that cannot change the type Caliper reports, and give the reason. An entry whose row stops differing, or names no row, fails the run, so the list stays honest: remove it. The list is empty while every row is identical.

`tests/construct-parity.test.ts` runs the same comparison in-process. It skips with a warning when `tsrs` is not installed; set `REQUIRE_TSRS=1` to fail instead.

## The checker lane

ADR 0026's gate before the default flips: the suite through the checker path (`--experimental-semantic-types` with the pinned `tsrs`), held to `checker-expectations.json`, which has the shape of `expectations.json`. Green rows must stay green; every other row is a documented gap with a one-line reason. The lane fails when a row is better or worse than its entry (an improvement means the gap closed: update the entry and drop the `gap`), when the checker did not answer for a row, or when the backend was unavailable.

```sh
just test-checker-lane   # CI job "Test / Checker lane"
```

The recipe builds the release binary, then runs `construct:check`, `construct:parity`, the lane (`construct:checker`) and `tests/construct-checker-lane.test.ts` with `REQUIRE_TSRS=1 REQUIRE_CALIPER_BIN=1`, so a missing binary fails instead of skipping. `TSRS_BIN` is the native binary the `tsrs` package itself selects (`src/tsrs-bin.ts` calls its `lib/getExePath.js`): `node_modules/@tsrs/<os>-<arch>[-gnu|-musl]/tsrs`. The `bin/tsrs` shim is a Node launcher and is not used. Regenerate an entry by running `CALIPER_SEMANTIC=1 TSRS_BIN=<path> pnpm --filter caliper-docgen-oracle construct:checker` and reading the report.

ADR 0026's S4 gate is that the checker path equals the structural path except listed improvements. The lane and `tests/construct-checker-lane.test.ts` (which needs no binary, so it runs with the package tests) hold the two files to it: a row whose checker verdicts rank below its `expectations.json` verdicts fails unless its `checker-expectations.json` entry gives an `"improvement"` reason, the case where the comparison scores the checker's answer lower although it states more (no row needs one today: `react/34` did until the comparator read `Omit<HTMLAttributes<…>>` as one family). An `"improvement"` on a row that does not rank below the structural path fails too, so the list stays honest. Rows the checker ranks above the structural path are improvements already explained by the structural `gap`. A member keyed by a symbol (`[tag]`) is read by `src/caliper-norm.ts` as the checker side states it.

## Not covered

Constructs the fixture format (one self-contained file, one prop `p`, no runtime code) cannot express, or that were left out:

- `declare const enum`: ambient const enums error under `isolatedModules` (TS2748); a local `const enum` is `literals/19`.
- Computed enum members (`E.A = 1 << 2`): left out to stay on common spellings.
- Computed property names from a runtime `const` or enum: needs runtime code; only literal keys and `declare const` unique-symbol keys are covered (`literals/24`, `objects/23`).
- Cross-file declaration merging and module augmentation: a fixture has no imports beyond React types; same-file merging is `objects/30`.
- DOM-typed `this` parameters: a plain object `this` type is used (`objects/18`, `generics/18`).
- `as const` value-derived tuples: needs runtime code; `declare const` with `typeof` covers it (`tuples-arrays/32`, `utility-mapped/22`).
- `ReactNode[]`: belongs to the React family's scope, not arrays.
- Generic components instantiated by an alias at JSX level: covered at type level by `generics/13` and `generics/16`.
- The global `JSX.Element` (removed in @types/react 19; `react/05` uses `React.JSX.Element`), `ReactChild`, `ReactChildren` and `LegacyRef` (removed or unstable across @types/react versions).
- `forwardRef` / `memo` calls wrapping the component: a fixture exports a plain `Row`; only the resulting exotic types are covered (`react/31`, `react/32`).

Fixtures that emit runtime code regardless: the enum fixtures (`literals/17` to `literals/23`) and the two shadowing fixtures (`generics/24`, `generics/25`), which need a local in the function body.

Dropped as invalid: `literals/26-union-unknown` and `literals/27-union-any`. The checker absorbs the union to `unknown` / `any`, so the golden has no type (`checker any/error`) and the row could not fail.
