---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: patch
---

- **Each diagnostic is reported once.** Identical diagnostics from many components merge into one, with a new `affects` field listing those components' output keys. Mantine's output drops from 24,009 diagnostics to 563 and from 24 MB to 15.6 MB; chakra-ui drops from 4,538 to 583 diagnostics.
- **Diagnostics are sorted** by file, line, column and message, with errors before warnings at the same spot.
- **`IncrementalUpdate.diagnostics`** holds the changed file's diagnostics plus those of every re-resolved component.
- **CLI:** the `×N` counts still count affected components.
- **Help text:** indexed-access and template-literal diagnostics no longer tell users to "enable typescript-go", which is not an option.
- **Breaking (Rust API):** `Diagnostic` has a new public `affects` field, so struct literals need it; `Diagnostic::new` is unaffected. `Diagnostic`, `DiagnosticSeverity` and `DiagnosticCode` now implement `Eq` and `Hash`.
