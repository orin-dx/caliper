# Archive

Historical record, not documentation. Nothing here reflects current state — check `docs/STATUS.md` and `docs/rdt-coverage.md` for that instead.

- `00-MASTER-PLAN.md` through `07-AGENT-DISPATCH.md` — the original agent work orders used to bootstrap this repo (Phases 0-5b). All complete.
- `08-OPEN-QUESTIONS.md` — pre-implementation design questions. Resolved.
- `10-QUALITY-AND-ARCHITECTURE.md` — the resolver/extractor module-split refactor plan. Done; see `ARCHITECTURE.md` for the resulting layout.
- `10-PLUGIN-SPEC.md` — the Vite plugin spec. Shipped; see README.md.
- `adversarial-analysis-2026-06-28.md` — a point-in-time validation pass. Findings folded into `docs/rdt-coverage.md`.
- `2026-06-16-phase-5a-vite-plugin.md` — the Vite plugin implementation plan (follows on from `10-PLUGIN-SPEC.md`). Shipped; see `packages/vite-plugin`.
- `2026-06-25-phase6-integration-tests.md` — wiring `moon run validate:compare` for `apps/validate`. Shipped; see `docs/rdt-coverage.md` for how it's used.
- `2026-06-28-structural-gap-fixes.md` — five resolver gaps found during RDT compatibility auditing (method-sig params, React namespace types, `Readonly`, inline `Pick`/`Omit`). All fixed.
- `2026-07-07-adversarial-analysis-fixes.md` — an eight-task wave from a five-domain (perf/Rust-best-practices/security/UX-DX/output-correctness) adversarial analysis. All eight landed. Two items it explicitly deferred (scoped-key allocation caching, `cache.rs` dirty-flag/size-cap) are folded into `docs/STATUS.md`'s "What's not built yet" section so they aren't lost.
- `2026-08-04-edge-case-remediation-plan.md` — the implementation plan for the 11 root causes and 7 findings in `docs/root-cause-analysis.md`. Done; produced ADR 0005 (panic containment) among other fixes.
- `2026-08-12-sdd-spec-drift-session-handoff.md` — a spec-driven-development pass that built the original `.claude/semantic-model/` reference layer, drafted `spec@1` documents, and fixed drift findings. Superseded — the semantic model now lives in `docs/architecture/`.
