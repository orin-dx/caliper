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

# Check the public Rust API of every published library crate for SemVer breaks against the last published version.
# The version only moves in the release PR, so a break is accepted when a pending minor or major changeset covers the
# crate. The CLI crate is a binary and has no library API.
semver:
    #!/usr/bin/env bash
    set -uo pipefail
    status=0
    for crate in caliper-docgen-core caliper-docgen-semantic caliper-docgen-tsc-client; do
        if cargo semver-checks check-release -p "$crate"; then
            continue
        fi
        if grep -qsE "^cargo/$crate: (minor|major)$" .changeset/*.md; then
            echo "breaking API changes are covered by a pending minor or major changeset for $crate"
            continue
        fi
        echo "breaking API changes need a minor or major changeset for cargo/$crate" >&2
        status=1
    done
    exit "$status"

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

# Run all tests (Rust, the native binding crate, TypeScript, Python)
test: test-rust test-napi test-ts test-py

# Run Rust tests (via nextest, which doesn't run doctests — paired
# with `cargo test --doc` to cover those too)
test-rust:
    cargo nextest run --workspace --exclude caliper-docgen-napi --locked
    cargo test --doc --workspace --exclude caliper-docgen-napi

# The binding crate is excluded from the workspace recipes, so its own unit tests run here.
test-napi:
    cargo nextest run -p caliper-docgen-napi --locked

# Run the Python tests for the weekly and agent-eval scripts (standard library only, no dependencies)
test-py:
    python3 -m unittest discover -s scripts -p 'test_*.py'

# Run TypeScript tests
test-ts:
    pnpm --filter caliper-docgen-vite-plugin test
    pnpm --filter caliper-docgen-storybook test
    pnpm --filter caliper-docgen-oracle test

# Compare the Go checker and tsrs on every construct-suite row (run on every tsrs bump; see apps/oracle/construct-suite/README.md)
test-parity:
    pnpm --filter caliper-docgen-oracle construct:parity

# The checker gates of ADR 0026, with nothing allowed to skip: the golden check, tsrs-against-Go parity, and the construct
# suite through the checker path held to checker-expectations.json. The pinned tsrs binary is the one the `tsrs` package
# itself selects for this platform (@tsrs/<os>-<arch>[-gnu|-musl]); see apps/oracle/construct-suite/README.md.
test-checker-lane:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --locked -p caliper-docgen
    export REQUIRE_TSRS=1 REQUIRE_CALIPER_BIN=1 CALIPER_BIN="$PWD/target/release/caliper-docgen"
    TSRS_BIN="$(pnpm --silent --filter caliper-docgen-oracle exec tsx src/tsrs-bin.ts)"
    export TSRS_BIN
    echo "tsrs: $TSRS_BIN"
    pnpm --filter caliper-docgen-oracle construct:check
    pnpm --filter caliper-docgen-oracle construct:parity
    CALIPER_SEMANTIC=1 pnpm --filter caliper-docgen-oracle construct:checker
    pnpm --filter caliper-docgen-oracle exec vitest run tests/construct-checker-lane.test.ts
    # S5's gate on tsrs: generic components instantiated (Test / Rust runs the same tests on the Go checker).
    cargo test --locked -p caliper-docgen-semantic --lib instantiate
    cargo test --locked -p caliper-docgen-semantic --test instantiate
    # S6's gate on tsrs: a kept snapshot brought up to date, and an edit nobody announced caught loudly.
    cargo test --locked -p caliper-docgen-semantic --test watch

# Build the native binding and run the Vite plugin against it
test-integration:
    pnpm --filter caliper-docgen-napi build:napi
    NAPI_BINARY_REQUIRED=true pnpm --filter caliper-docgen-vite-plugin test:integration
    NAPI_BINARY_REQUIRED=true pnpm --filter caliper-docgen-storybook test:integration

# Measure ADR 0026's memory, determinism, first-answer and watch gates against the pinned tsrs
# (docs/gate-measurements.md). Builds the large project outside the repository (GATE_FIXTURES, default
# ~/Projects/caliper-fixtures/gate-large), so it needs the network or a warm pnpm store; `just gate-measure --runs 3`
# passes options to every gate. Takes minutes, runs every gate, and fails if any failed.
gate-measure *args:
    #!/usr/bin/env bash
    set -euo pipefail
    fixtures="${GATE_FIXTURES:-$HOME/Projects/caliper-fixtures/gate-large}"
    node scripts/gate_fixtures.mjs "$fixtures"
    cargo build --release --locked -p caliper-docgen-semantic --example gate_harness
    TSRS_BIN="$(pnpm --silent --filter caliper-docgen-oracle exec tsx src/tsrs-bin.ts)"
    failed=0
    for gate in memory determinism first-answer watch; do
        target/release/examples/gate_harness "$gate" --src "$fixtures/src" --tsc "$TSRS_BIN" {{ args }} || failed=1
    done
    exit "$failed"

# The deterministic construct suite on the release binary: goldens current, then the suite, which must not skip
test-construct:
    cargo build --release -p caliper-docgen --locked
    CALIPER_BIN="$PWD/target/release/caliper-docgen" pnpm --filter caliper-docgen-oracle construct:check
    CALIPER_BIN="$PWD/target/release/caliper-docgen" REQUIRE_CALIPER_BIN=1 pnpm --filter caliper-docgen-oracle test

# Pack the npm packages and check napi arrives only as vite-plugin's transitive dependency (pm: npm or pnpm)
verify-install pm="pnpm":
    scripts/verify-install.sh {{ pm }}

# Install the packed CLI and napi packages as a release lays them out, with no TypeScript and no TSRS_BIN, and check
# both find the pinned tsrs installed beside them and that it answers (pm: npm or pnpm; the built CLI binary and addon)
verify-checker-install pm cli addon:
    scripts/verify-checker-install.sh {{ pm }} {{ cli }} {{ addon }}

# Run the criterion benchmarks (single fixtures); `just bench` compares a whole library against main
bench-criterion:
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

# TypeScript coverage: prints the summaries and writes each package's coverage/lcov.info
coverage-ts threshold="":
    pnpm --filter caliper-docgen-vite-plugin exec vitest run --coverage {{ if threshold != "" { "--coverage.thresholds.lines=" + threshold } else { "" } }}
    pnpm --filter caliper-docgen-storybook exec vitest run --coverage {{ if threshold != "" { "--coverage.thresholds.lines=" + threshold } else { "" } }}

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

ts_src := "'packages/vite-plugin/src/**/*.ts' 'packages/vite-plugin/tests/**/*.ts' 'packages/vite-plugin/vitest.config.ts' 'packages/storybook/src/**/*.ts' 'packages/storybook/tests/**/*.ts' 'packages/storybook/*.config.ts' 'packages/napi/*.d.ts' 'packages/cli/bin/**/*.js' 'apps/oracle/src/**/*.ts' 'apps/oracle/tests/**/*.ts'"
ts_src_only := "'packages/vite-plugin/src' 'packages/vite-plugin/vitest.config.ts' 'packages/storybook/src' 'packages/storybook/vitest.config.ts' 'packages/napi/*.d.ts' 'packages/cli/bin/**/*.js' 'apps/oracle/src'"
ts_tests_only := "'packages/vite-plugin/tests' 'packages/storybook/tests' 'apps/oracle/tests'"
md_src := "'**/*.md' '!node_modules/**' '!target/**' '!dist/**'"

# Lint and format check
lint:
    cargo fmt --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    node scripts/check-packaging.mjs
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
    cargo clippy --workspace --all-targets --locked -- -D warnings
    typos
    cargo deny check

# Everything the CI jobs run, minus the Node version matrix and verify-install
ci: lint test deny typos zizmor doc-check machete semver msrv (coverage "90")

# Time and memory of this checkout (or a branch) against origin/main on Mantine and Chakra; see docs/measurement.md.
bench *args:
    python3 scripts/bench.py {{args}}

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
