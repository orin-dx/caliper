# Contributing

By participating in this project, you agree to abide by the [Code of Conduct](./CODE_OF_CONDUCT.md). Found a security issue? See [SECURITY.md](./SECURITY.md) instead of opening a public issue.

## Prerequisites

| Tool  | Min version   | Notes                                         |
| ----- | ------------- | --------------------------------------------- |
| Rust  | 1.100         | `rustup update stable`                        |
| proto | latest        | manages Node/pnpm versions from `.prototools` |
| pnpm  | `.prototools` | via proto, or `npm i -g pnpm`                 |

After installing proto, run `moon setup` once to pull the pinned Node version and any other tool versions declared in `.prototools`.

## Build & test

```bash
# Rust
cargo build                                            # dev build
cargo test -p caliper-docgen-core                   # unit + snapshot tests
cargo clippy -p caliper-docgen-core -- -D warnings
cargo fmt --all

# TypeScript
pnpm --filter caliper-docgen-vite-plugin test      # unit tests (mocked NAPI)
pnpm --filter caliper-docgen-vite-plugin test:integration  # integration tests (real native binding)

# Speed and memory of your branch against origin/main (Mantine and Chakra)
just bench
```

The pre-commit hook runs `cargo fmt`, `cargo clippy --all-targets -D warnings`, `typos`, and `cargo deny` on every commit. It's fast enough that skipping it isn't worth the trouble.

## Local disk, caches and worktrees

Each place holds one kind of thing, and its name says which.

- **The repository:** code we re-run (`scripts/`, `apps/oracle`), decisions (`docs/adr/`), process (this file, `AGENTS.md`), and small test inputs (`fixtures/`). Anything a later change or a reviewer needs goes here, or in the pull request description.
- **Caches you can delete at any time:** `target/`, `node_modules/`, and `.corpora/` (the weekly's pinned repos, gitignored; `docs/measurement.md`). Deleting one costs rebuild or download time and nothing else. A worktree's `target/` was about 5 GB and its `node_modules/` about 400 MB when this was written.
- **Throwaway output:** logs, A/B binaries, diffs. Write them under `$TMPDIR`, which is not kept. If you need a result later, record it in the pull request or in `docs/`.
- **Nothing outside the repository is kept for the project.** Do not create a directory under `~/Projects` for output, and do not store build output in a directory named for something else (a `fixtures` directory holds test inputs only).

`just test-checker-lane` builds into `target/release` of the current checkout and reads the binary from there, so run it without `CARGO_TARGET_DIR` set.

Each worktree has its own `target/`, `node_modules/` and `.corpora/`. To share one copy of the corpora between worktrees, pass `--corpora DIR` to `scripts/corpora.py` and `scripts/weekly.py`. Keep few worktrees: after a branch is merged and pushed, remove its worktree (`git worktree remove`), delete the local branch, and run `git worktree prune`.

## Before you push a behavior change

`just lint doc-check machete typos` must pass, plus the tests of the crates you touched. CI runs the rest. A change that decides something about types or props (relevance, resolution, merging) also needs:

- **Invariants:** state them in the pull request description: unknown unless proven, by binding and not by spelling, every read recorded, one owner for each decision, wired or not defined.
- **Allow-lists:** a guard handles a closed set of proven forms, and everything else is unknown or declined, never "complete", "none" or "exact". No `_ =>` arm in a decision over node or type kinds.
- **Grammar sweep:** list every syntax or type form the code can receive and show each is handled or declined. Prefer a generated test over the grammar with the TypeScript checker as the oracle (`crates/semantic/tests/`) to a hand-picked list. Cover lone surrogates, computed keys, shadowing, `declare`, overloads and re-exports.
- **Mutation proof:** remove each guard in turn; a test must fail. `cargo mutants --in-diff <diff>` finds the ones no test notices.
- **A test that fails on `main`** for every behavior change.
- **Output-preserving change:** diff `extract --json --quiet --no-cache` against a build of `main` on a large project and on Chakra, with `stats` removed. For resolver or checker changes, also compare peak memory and time with `just bench`, because a size regression can pass every test.
- **A request to the checker:** a test that each parameter you send changes the answer. `tsrs` ignores keys it does not know, so a misspelled field compiles and passes (`file` for `files` made every cold run type-check the whole program).
- **Measuring:** interleave the builds and compare CPU time as well as wall time. Drop any run that carries a checker fallback diagnostic: a timed-out request falls back to the structural answer, so that run's output differs. Benchmark a combination of changes together; estimates measured separately do not add.
- **Findings:** when an automated or human review finds a defect, fix the class it belongs to, not only the case, and resolve the finding with the commit and test that fix it.

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

**`crates/binding`** — thin NAPI wrapper. Owns the async runtime (tokio) and exposes the NAPI functions listed in `packages/napi/binding.d.ts` (batch extraction, sessions, and the lazy session's index, answers and warm-up). No business logic.

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

- GPG-signed, `Signed-off-by` trailer (both configured in `.git/config`). `main` requires signed commits, so an unsigned commit in a pull request blocks the merge.
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

## Releases

Add a changeset for a consumer-visible change (`.changeset/`; see `AGENTS.md` for how to write one). Callisto (version pinned in `.github/workflows/callisto-release.yml`) plans the versions from the changesets and opens a release pull request (`callisto/version-packages`); merging it runs `.github/workflows/callisto-release.yml`, which publishes the crates and npm packages, tags each package, and publishes one GitHub Release for the CLI (`cargo/caliper-docgen@x.y.z`) with its changelog section as the notes and the five CLI binaries attached, each with a build attestation. The npm wrapper's tag `npm/caliper-docgen@x.y.z` is what the tap reads. `docs/releasing.md` has the release flow and how to recover a failed run.

Homebrew pulls from us: the tap's daily `brew bump` finds that tag with `livecheck`, builds `crates/cli` from source and bottles it, and a maintainer merges the formula pull request. Nothing in this repository notifies the tap.

Speed, memory and accuracy are recorded weekly and compared with the previous run (`docs/measurement.md`); there is no pull-request gate. Run `just bench` when a change touches the resolver.

## Pull requests

1. Branch from `main`
2. Keep the pre-commit hook passing
3. Update snapshots if extraction output changes (`cargo insta review`)
4. One logical change per PR — squash fixup commits before merging

> `just bench` is a local check of speed and memory against `main`; it prints a table and fails nothing. Accuracy against RDT and react-docgen is tracked by the weekly record (`docs/measurement.md`), not by a pull-request gate.
