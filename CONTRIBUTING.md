# Contributing

By participating in this project, you agree to abide by the [Code of Conduct](./CODE_OF_CONDUCT.md). Found a security issue? See [SECURITY.md](./SECURITY.md) instead of opening a public issue.

## Prerequisites

| Tool  | Min version | Notes                                         |
| ----- | ----------- | --------------------------------------------- |
| Rust  | 1.97        | `rustup update stable`                        |
| proto | latest      | manages Node/pnpm versions from `.prototools` |
| pnpm  | 9           | via proto, or `npm i -g pnpm`                 |

After installing proto, run `moon setup` once to pull the pinned Node version and any other tool versions declared in `.prototools`.

## Build & test

```bash
# Rust
cargo build                                            # dev build
cargo test -p caliper-docgen-core                   # unit + snapshot tests
cargo clippy -p caliper-docgen-core -- -D warnings
cargo fmt --all

# TypeScript
pnpm --filter caliper-docgen-vite-plugin test      # 33 unit tests (mocked NAPI)
pnpm --filter caliper-docgen-vite-plugin test:integration  # 3 integration tests (real native binding)

# Speed and memory of your branch against origin/main (Mantine and Chakra)
just bench
```

The pre-commit hook runs `cargo fmt`, `cargo clippy --all-targets -D warnings`, `typos`, and `cargo deny` on every commit. It's fast enough that skipping it isn't worth the trouble.

## Snapshot tests

Snapshot tests use [insta](https://insta.rs). Each fixture (shadcn, MUI, Chakra, etc.) has a snapshot of the full extraction output. If your change touches extraction output, insta will flag it:

```bash
cargo test -p caliper-docgen-core    # fails with "snapshot mismatch"
cargo insta review                     # review diffs, accept or reject
```

Accept any snapshot change that is intentional (i.e., the output got more accurate). Reject regressions.

## Where code goes

**`crates/core`** — everything that understands TypeScript ASTs and React component patterns. No I/O, no async, no terminal output. Every function here must be `Send + Sync` so rayon can parallelize it.

**`crates/cli`** — user-facing output only. No extraction logic. Commands parse args and delegate to `core::pipeline`. Error formatting uses `miette`; progress uses `indicatif`; table output uses `comfy-table`.

**`crates/binding`** — thin NAPI wrapper. Owns the async runtime (tokio) and exposes the five NAPI functions (`extractAll`, `createSession`, `initializeSession`, `extractFileIncremental`, `closeSession`). No business logic.

**`packages/napi`** — `caliper-docgen-napi`. TypeScript types (`index.d.ts`) and the dev binary loader (`index.js`) that finds the `.node` file from `target/` or `NAPI_RS_NATIVE_LIBRARY_PATH`. No logic.

**`packages/vite-plugin`** — `caliper-docgen-vite-plugin`. Single Vite `Plugin` object. No extraction logic — calls NAPI and manages HMR.

## Code style

The authoritative reference is `.claude/skills/rust-style/SKILL.md`. Highlights:

**Maps:** `FxHashMap` for internal maps (hot path, no DoS surface). `BTreeMap` for anything that ends up in JSON output (key ordering must be deterministic and stable across runs).

**Strings:** `CompactString` for type names and prop names. Most are ≤24 bytes (`string`, `boolean`, `ReactNode`, `MouseEvent`) and fit inline without heap allocation.

**Control flow:** `let...else` keeps the happy path flat:

```rust
let Some(entry) = map.get(key) else { return None };
```

**Matching on project-local enums:** no wildcards. `match status { Ok => .., Err => .. }` gives a compile error when new variants are added. `match status { Ok => .., _ => .. }` silently ignores them.

**No `unwrap()` outside tests.** Use `?` or emit a `Diagnostic` and return a safe default. The `Diagnostic` type has constructors for common error shapes.

**Comments:** only when the WHY is non-obvious. Not what the code does.

## Commits

- GPG-signed, `Signed-off-by` trailer (both configured in `.git/config`)
- `feat:` / `fix:` / `refactor:` / `chore:` / `docs:` prefix
- No line wrapping at any column
- Subject line says it all — skip the body if it adds nothing

```
# good
feat: resolve ComponentPropsWithoutRef in intersection type aliases

# bad
Updated the resolver to also handle the case where ComponentPropsWithoutRef
appears inside an intersection type alias, which was causing it to be
treated as opaque when it should expand the HTML attributes.
```

## Pull requests

1. Branch from `main`
2. Keep the pre-commit hook passing
3. Update snapshots if extraction output changes (`cargo insta review`)
4. One logical change per PR — squash fixup commits before merging

> `just bench` is a local check of speed and memory against `main`; it prints a table and fails nothing. Accuracy against RDT and react-docgen is tracked by the weekly record (`docs/measurement.md`), not by a pull-request gate.
