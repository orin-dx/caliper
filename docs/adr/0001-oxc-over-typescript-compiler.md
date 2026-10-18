# 0001. Parse with OXC instead of the TypeScript compiler

**Status:** Accepted **Date:** 2026-07-22 (retroactive — true since the project started)

## Context

`react-docgen-typescript` extracts props by spinning up a full TypeScript `Program` — real type-checking, not just parsing. On a mid-size design system that takes seconds, which shows up as slow Storybook startup and sluggish HMR. We wanted prop extraction fast enough to run on every file save without anyone noticing.

## Decision

Parse TypeScript/TSX with OXC — a Rust parser with no type-checking pass — and resolve prop types structurally from the AST instead of asking a type checker. Cold extraction on a 15-component fixture set (shadcn, MUI, Chakra, Mantine, React Aria, Radix) takes 32ms.

## Consequences

- Parsing runs in parallel per file with no shared `Program` state to build or invalidate — this is most of the speed win, not just OXC being a fast parser.
- We give up real type inference. Conditional types, mapped types over an unbound generic, and a few `@emotion/styled`-specific call shapes can't be resolved without one — see `docs/type-checker-integration.md` for the full list.
- Every one of those gaps degrades to a `Diagnostic` plus an `Opaque` prop type, never a silent wrong answer (`AGENTS.md` non-negotiable #6) — the speed trade-off costs completeness, never correctness.
- Opt-in type inference is planned as a second resolver on TypeScript 7's programmatic API, with the structural path staying the default. See ADR 0006 and `docs/type-checker-integration.md`.
