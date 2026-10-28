# Edge cases and known gaps

Gaps that are still open. The 2026-08-04 audit that this file used to record (crash and hang risks, silent data loss, silent wrong output; 43 fixes) has landed; its findings are in git history (`git log --follow docs/edge-cases.md`) and each fix has a regression test. Bugs found by real-library validation are in `rdt-coverage.md`. Type-inference gaps are in `type-checker-integration.md`.

Add an entry when you find a gap you are not fixing now. Delete it when you fix it.

## Detection

- Wrapper calls with no type argument are not recognized: `createIconLoader('x')`, `lazy(() => import(...))`, `createIcon({...})`. Reading the props needs the call's return type, which the structural path doesn't have. The opt-in TypeScript backend (ADR 0006) can.
- Some components are reported under a different name than `react-docgen-typescript` uses: `displayName` assignments, and compound members such as `Tabs.List`.

## Resolution

- Mapped types, conditional types and indexed access over an unbound generic, and `@emotion/styled`'s two-argument overload, need real type inference. They degrade to `Opaque` with a diagnostic and, where possible, a `composes` entry. See `type-checker-integration.md`.
- Ambient globals (`lib.dom.d.ts` and the like) are looked up from `options.src_dirs.first()` only (`pipeline/mod.rs`, `pipeline/revision.rs`). If the first source directory can't resolve `typescript` but a later one could, ambient lookups return nothing for the whole run. Untested against a real multi-root config.

## Output

- TOON is lossy on purpose: unions and intersections print at most 4 members, literal unions at most 6, each followed by `...(+N)`. The module doc of `toon.rs` doesn't say so.
