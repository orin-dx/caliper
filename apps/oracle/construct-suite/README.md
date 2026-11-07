# Deterministic construct suite

One small fixture per type construct, each compared with what TypeScript's checker says. Unlike the weekly oracle (`scripts/weekly.py --oracle`), it runs on a handful of files in seconds, needs no corpus, and fails on any change in how Caliper reads a construct.

## Layout

```
construct-suite/
  tsconfig.json              the options the checker reads the fixtures with
  expectations.json          row id -> allowed verdicts (and the reason for each gap)
  <family>/<NN>-<slug>.tsx   a fixture
  <family>/<NN>-<slug>.golden.json   its committed golden, written by construct:capture
```

A row id is `<family>/<NN>-<slug>`. The harness is `../src/construct-suite.ts` (pure helpers), `../src/construct-capture.ts` (the capture script) and `../tests/construct-suite.test.ts`. `seed/` holds the three rows that prove the harness.

A fixture declares one `interface Props` with one prop `p`, and exports a component `Row` that takes it:

```tsx
interface Props {
  p: 'small' | 'large'
}

export function Row(props: Props) {
  return null
}
```

## Commands

```sh
# Write the golden files: the checker (typescript-beta, the independent arbiter) over all fixtures in one program.
pnpm --filter caliper-docgen-oracle construct:capture

# Re-capture and fail on any difference from the committed goldens. Run it when the TypeScript pin changes.
pnpm --filter caliper-docgen-oracle construct:check      # same as: construct:capture --check

# The suite itself, with the package tests.
pnpm --filter caliper-docgen-oracle test
```

The test runs one `extract` over this directory with the weekly's canonical options (`--html-attributes full --include-unexported --load-dependencies --collapse-inherited-props none`, plus `--no-cache --quiet`) and compares prop `p` of each `Row` with its golden. The binary is `$CALIPER_BIN`, or `target/release/caliper-docgen`. Without one the suite skips with a message and the rest of the package tests still pass.

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

A row is green or a documented gap. A gap lists the verdicts it may have, none of them green, and a `gap` reason. The test fails when a row moves either way: worse is a regression; better means the gap closed, so update the expectation (to green, dropping `gap`). A gap never allows `identical` or `spelled`, so an improvement cannot pass unnoticed.

## CI

The suite needs Caliper's release binary. A CI job should build it before the package tests and point the test at it:

```sh
cargo build --release -p caliper-docgen
CALIPER_BIN="$PWD/target/release/caliper-docgen" pnpm --filter caliper-docgen-oracle test
pnpm --filter caliper-docgen-oracle construct:check
```

Without the build step the suite skips, which is green but covers nothing; a job that must run it should check that the binary exists first.
