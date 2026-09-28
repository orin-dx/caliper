# Security policy

## Reporting a vulnerability

Email **security@orin-dx.com** — don't open a public issue for anything that could be exploited before a fix ships.

Include:

- Which crate or package is affected (`crates/core`, `crates/binding`, `crates/cli`, `@oxc-react-docgen/napi`, `@oxc-react-docgen/vite-plugin`, `@oxc-react-docgen/cli`)
- The concrete failure scenario — what an attacker could do, and how
- A minimal reproduction, ideally a TSX/JSX snippet if the issue is parser-triggered

Expect an acknowledgment within 5 business days. We'll keep you posted as a fix moves through triage, and credit you in the release notes unless you'd rather stay anonymous.

## Scope

This tool parses TypeScript/TSX source to extract React component props. The realistic threat model:

- **Malicious or adversarial source input.** `crates/core` and `crates/cli` build with `#![forbid(unsafe_code)]`, so a crafted `.tsx` file causing memory corruption there would be a serious finding. More likely and still real: a crafted file causing a panic, unbounded memory growth, or a hang (e.g. pathological recursive types, extremely deep JSX nesting) when run against untrusted input — for example in CI pipelines or editor tooling that extract props from user-submitted code.
- **`crates/binding`** is the NAPI wrapper and has no `unsafe_code` forbid attribute — it's the one place in this codebase where FFI-boundary unsafety can legitimately exist (napi-rs's generated glue, not necessarily hand-written `unsafe` blocks in this crate). A bug here that's reachable from crafted TS/TSX input, not just from a trusted host application, is in scope.
- **Supply chain**, both sides: the Cargo dependency tree (`oxc_*`, `napi`, etc.) and the npm packages once published (`@oxc-react-docgen/napi`, `@oxc-react-docgen/vite-plugin`, `@oxc-react-docgen/cli`) — the CLI package execs a platform binary rather than reimplementing logic, so a compromised binary distribution channel is a real category once these ship.
- **The Vite plugin's dev-server surface** — `virtual:oxc-react-docgen` and HMR push component metadata into a running dev server; a way to inject unintended content through that channel is in scope.

Out of scope: accuracy gaps in prop extraction (wrong or missing props for a valid pattern) — those are correctness bugs, file them as a normal issue, not a security report.

## Supported versions

This project is pre-1.0 and not yet published to npm. Security fixes land on `main`; there's no older release line to backport to yet. Once versioned releases ship, this section will name which lines receive fixes.
