# 0007. Let the backend discover components, always compile it in, and keep structural as the default

**Status:** Accepted **Date:** 2026-10-21

## Context

ADR 0006 made the TypeScript backend a second resolver that finishes components the structural resolver already found. Detection by shape now covers 92 to 99% of the components react-docgen-typescript (RDT) reports on five large libraries (`docs/edge-cases.md`), and what is left is a call with no type argument (`lazy()`, `createIconLoader("x")`) or a prop a callee adds (Mantine's `component` and `renderRoot` on 220 components). Only the checker can answer those: it knows what the call returns. Separately, callisto's rule is no cargo features in shipped crates, which ADR 0006's "feature-gated crate" contradicts.

## Decision

- **Discovery.** The backend may add components the structural pass did not find, using RDT's rule: a declaration whose type has a call signature with a props-shaped first parameter. Each carries the rule that found it and merges into the structural output; on a clash the structural entry wins.
- **Always compiled, enabled at runtime.** `crates/tsc-client` is a normal dependency of the CLI and the NAPI addon, with no cargo feature. A runtime option turns it on and is off by default, so a run without it spawns nothing.
- **Structural stays the default and the fallback.** A missing, unsupported or crashing `tsc` leaves the structural result untouched and adds a diagnostic.
- **Trigger.** The backend runs only for components the structural pass marked incomplete (an opaque leaf, a wrapper call with no resolved props, a callee-added prop it cannot see) plus whole-project discovery when the option asks for it. It is never a silent default.
- **Output.** `ExtractionOutput` gains an additive `schemaVersion` so consumers can tell whether discovery was on. Existing fields keep their shape.

## Consequences

- Wrapper calls the structural pass cannot see become exact without a new heuristic per library.
- Every build pays for the client's dependencies. Users who never enable the backend pay nothing at runtime.
- The checker needs `typescript` 7.1 or later and sees a dependency edit only when told. `WatchSession::update_file` must notify it, with a parity test against `extract()`.
- Checker types do not fit `PropType` losslessly, so a checker answer is converted at the boundary and anything that does not fit stays opaque with a reason.
- Determinism needs a gate: a clean run and an incremental run must agree, measured on the five corpora with dependencies installed.
- Supersedes only the feature-gating bullet of ADR 0006. The rest of 0006 stands.

## Alternatives considered

- **Keep the backend a feature of a separate crate.** Matches ADR 0006 as written but breaks the no-features rule and makes the shipped binary vary by build.
- **Run it by default.** Costs a process per run on projects that structural detection already handles and makes output depend on a toolchain being installed.
- **Add one structural heuristic per remaining shape.** The remaining shapes need the callee's return type; a heuristic per library is what the shape-based detector was built to avoid.
