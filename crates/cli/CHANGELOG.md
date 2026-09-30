# oxc-react-docgen

## 0.1.1

- Lint policy (`unsafe_code`, clippy `all`) moved from per-crate `#![...]` attributes into a single `[workspace.lints]` table in the workspace root, with each crate opting in via `[lints] workspace = true`. `unsafe_code` is now `"deny"` rather than `"forbid"` — no crate currently uses `unsafe`, including the NAPI FFI boundary, but `deny` allows a local `#[allow(unsafe_code)]` override if one is ever genuinely needed there.
  
  Also dropped 11 dependencies that `cargo-machete` confirmed were unused: `indexmap`, `oxc_module_lexer`, `thiserror`, `tokio`, `dirs`, `miette` (core); `anyhow`, `clap_mangen`, `lsp-types`, `tracing-indicatif` (cli); `rustc-hash` (binding). No behavior change — smaller dependency graph and faster builds.
- Fixed 7 doc-comment errors that broke `cargo doc` under `-D warnings`: 6 unescaped angle-bracket type names (`Ref<T>`, `RefObject<T>`, `HTMLButtonElement`, etc.) that rustdoc parsed as unclosed HTML tags, and one `[default]` that rustdoc parsed as a broken intra-doc link. No behavior change — fixes how the published crate's documentation renders on docs.rs.
- An unrecognized `htmlAttributes` value (a typo in `docgen.config.ts` or in the JS options) is now an error naming the value, instead of silently falling back to `curated`, as `reactVersion` already does.
- `--format rdt` omits a `children` prop that has no description, matching react-docgen-typescript's default `skipChildrenPropWithoutDoc`. A documented `children` is still emitted, and `--format canonical` is unchanged.
- `watch` no longer spins a CPU core when stdin isn't a terminal, and ends on SIGINT, SIGTERM, SIGHUP and SIGQUIT instead of ignoring them, restoring the terminal and exiting with the session's exit code. Only Ctrl-C quits from the keyboard, not a bare `c`.
- Joined the `cli-and-npm-wrapper` group at this version.

