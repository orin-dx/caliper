# Edge cases and known gaps

Gaps that are still open. The 2026-08-04 audit that this file used to record (crash and hang risks, silent data loss, silent wrong output; 43 fixes) has landed; its findings are in git history (`git log --follow docs/edge-cases.md`) and each fix has a regression test. Bugs found by real-library validation are in `rdt-coverage.md`. Type-inference gaps are in `type-checker-integration.md`.

Add an entry when you find a gap you are not fixing now. Delete it when you fix it.

## Detection

- Wrapper calls with no type argument are not recognized: `createIconLoader('x')`, `lazy(() => import(...))`, `createIcon({...})`. Reading the props needs the call's return type, which the structural path doesn't have. The opt-in TypeScript backend (ADR 0006) can.
- Some components are reported under a different name than `react-docgen-typescript` uses: `displayName` assignments, and compound members such as `Tabs.List`.

## Resolution

- Mapped types, conditional types and indexed access over an unbound generic, and `@emotion/styled`'s two-argument overload, need real type inference. They degrade to `Opaque` with a diagnostic and, where possible, a `composes` entry. See `type-checker-integration.md`.
- Tuple element resolution has known gaps. An optional tuple member (`[a?: T]`) renders as its bare type, with no optional marker and no `undefined`. A conditional type in a tuple element whose type parameter shadows an alias of the same name (`[T extends string ? 1 : 2]` inside `function Box<T>`) resolves through the alias (`[1]`) instead of staying a conditional. A factory with a generic type argument (`Field = make<Props<string>>()` for `make<Props>(): React.FC<Props>`) resolves its props here; the scoping interaction a review reported for it is not handled. A fuller scoping attempt is on the `wip/css-tuple-full` branch.
- Ambient globals (`lib.dom.d.ts` and the like) are looked up from `options.src_dirs.first()` only (`pipeline/mod.rs`, `pipeline/revision.rs`). If the first source directory can't resolve `typescript` but a later one could, ambient lookups return nothing for the whole run. Untested against a real multi-root config.

## Watch and polling

- Edits to dependency declarations outside the source directories are not noticed by a watching session, by `update_file` or by `ensure_fresh`. This covers a workspace package that `node_modules` links to by symlink: dependents keep the old types until the session restarts or a resolution config changes. Neither call knows which dependency files a revision read, `ensure_fresh` stamps only source files, directories and resolution configs, and a host watcher is only told about the source directories. Following them needs the loaded dependency files exposed to the stamps and to the host's watch list, plus a rebuild of the dependency layer on change. Reproduced with a symlinked `ws-kit` package: after editing its `index.d.ts`, `ensure_fresh` and `update_file` leave the old props.
- A file that a tsconfig `extends` is not watched unless its name looks like a resolution config (`tsconfig*.json`, `jsconfig.json`, `package.json`) and it sits in or above a source directory. Editing `configs/paths.json` in `{ "extends": "./configs/paths.json" }` keeps the old path aliases until the session restarts. Finding the chain needs the extends resolution `oxc_resolver` does internally (relative paths, package names, arrays). Reproduced with `ensure_fresh`.
- Symlinked directories under a source directory are not followed (the walker does not follow links), and no diagnostic says so. Files behind them are never discovered. Following them is out of scope; a diagnostic naming the skipped link is not yet emitted.
