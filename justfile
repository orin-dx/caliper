# List available commands
default:
    @just --list

# Install Cargo tools required for local development
install-tools:
    cargo install cargo-nextest --locked
    cargo install cargo-llvm-cov --locked
    cargo install cargo-machete --locked
    cargo install cargo-mutants --locked
    cargo install cargo-semver-checks --locked
    rustup component add llvm-tools-preview

# Check the public Rust API of the core crate for SemVer breaks against the last published version.
semver:
    cargo semver-checks check-release -p caliper-docgen-core

# Build with the minimum supported Rust version declared in Cargo.toml.
msrv:
    cargo +$(sed -n 's/^rust-version = "\(.*\)"/\1/p' Cargo.toml) check --workspace --exclude caliper-docgen-napi --locked

# Install all dependencies
install:
    pnpm install
    cargo fetch

# Build all projects. Rust crates build via one workspace-wide cargo
# invocation, not `moon run :build` — that fans out one `cargo build -p`
# per crate, and every one of those processes contends for the same
# target/ file lock, serializing what moon's own scheduler thinks is
# parallel work (confirmed via `Blocking waiting for file lock` in verbose
# output). napi/vite-plugin have no such lock to contend over, so moon
# still orchestrates those directly.
build:
    cargo build --workspace --exclude caliper-docgen-napi
    moon run napi:build vite-plugin:build
    cargo build --release

# Run all tests (Rust + TypeScript)
test: test-rust test-ts

# Run Rust tests (via nextest, which doesn't run doctests — paired
# with `cargo test --doc` to cover those too)
test-rust:
    cargo nextest run --workspace --exclude caliper-docgen-napi --locked
    cargo test --doc --workspace --exclude caliper-docgen-napi

# Run TypeScript tests
test-ts:
    pnpm --filter caliper-docgen-vite-plugin test

# Pack the npm packages and check napi arrives only as vite-plugin's transitive dependency (pm: npm or pnpm)
verify-install pm="pnpm":
    scripts/verify-install.sh {{ pm }}

# Run benchmarks
bench:
    cargo bench --workspace --exclude caliper-docgen-napi

# Rust and TypeScript line coverage; with a threshold, fails if either is below it
coverage threshold="": (coverage-rust threshold) (coverage-ts threshold)

# Rust coverage: writes lcov.info and prints the summary before enforcing the threshold
coverage-rust threshold="":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo llvm-cov nextest --workspace --exclude caliper-docgen-napi --locked --lcov --output-path lcov.info
    cargo llvm-cov report --summary-only
    if [[ -n "{{ threshold }}" ]]; then
      cargo llvm-cov report --summary-only --fail-under-lines "{{ threshold }}"
    fi

# TypeScript coverage: prints the summary and writes packages/vite-plugin/coverage/lcov.info
coverage-ts threshold="":
    pnpm --filter caliper-docgen-vite-plugin exec vitest run --coverage {{ if threshold != "" { "--coverage.thresholds.lines=" + threshold } else { "" } }}

# Rust coverage report, opened in the browser
coverage-html:
    cargo llvm-cov nextest --workspace --exclude caliper-docgen-napi --locked --html --open

# Documentation build check (warnings, including broken intra-doc links,
# treated as errors)
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --exclude caliper-docgen-napi

# Unused-dependency check
machete:
    cargo machete

# Mutation testing sweep — slow, run on demand, not part of `just ci`
mutants:
    cargo mutants --workspace --exclude caliper-docgen-napi

ts_src := "'packages/vite-plugin/src/**/*.ts' 'packages/vite-plugin/tests/**/*.ts' 'packages/vite-plugin/vitest.config.ts' 'packages/napi/*.d.ts' 'packages/cli/bin/**/*.js' 'apps/validate/src/**/*.ts'"
ts_src_only := "'packages/vite-plugin/src' 'packages/vite-plugin/vitest.config.ts' 'packages/napi/*.d.ts' 'packages/cli/bin/**/*.js' 'apps/validate/src'"
ts_tests_only := "'packages/vite-plugin/tests'"
md_src := "'**/*.md' '!node_modules/**' '!target/**' '!dist/**'"

# Lint and format check
lint:
    cargo fmt --check
    cargo clippy --workspace --exclude caliper-docgen-napi --locked -- -D warnings
    pnpm exec oxfmt --check {{ts_src}}
    pnpm exec oxfmt --check {{md_src}}
    pnpm exec oxlint {{ts_src_only}}
    pnpm exec oxlint -A pedantic -A perf -A style -A unicorn/consistent-function-scoping -A vitest/require-mock-type-parameters --vitest-plugin {{ts_tests_only}}

# Format code
fmt:
    cargo fmt
    pnpm exec oxfmt {{ts_src}}
    pnpm exec oxfmt {{md_src}}

# Check licenses, advisories, and banned crates
deny:
    cargo deny check

# Check for typos
typos:
    typos

# Lint GitHub Actions workflows for permission, credential, and supply-chain
# issues. --strict-collection fails on a workflow it cannot parse instead of
# skipping it, so a malformed file can never pass by being ignored.
zizmor:
    zizmor --offline --strict-collection .

# Everything the moon-managed pre-commit hook runs. `.moon/workspace.yml`
# calls this recipe rather than repeating the commands, so the hook and
# `just ci` can't drift apart.
pre-commit:
    cargo fmt --check
    cargo clippy --workspace --exclude caliper-docgen-napi --locked -- -D warnings
    typos
    cargo deny check

# Everything the CI jobs run, minus the Node version matrix and verify-install
ci: lint test deny typos zizmor doc-check machete semver msrv (coverage "90")

# Run moon compare task (accuracy vs react-docgen + react-docgen-typescript)
compare:
    moon run validate:compare

# Clean build artifacts
clean:
    moon clean
    cargo clean
    rm -rf node_modules

# Setup toolchain via proto
setup:
    proto install
    pnpm install
    moon sync vcs-hooks

# Run moon commands
moon *args:
    moon {{args}}

# Run cargo commands
cargo *args:
    cargo {{args}}
