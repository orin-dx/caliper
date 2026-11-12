---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-storybook: patch
---

The npm packages now install the TypeScript checker Caliper pins, `tsrs` 0.23.0-ts7.1.0-beta, as an optional dependency of each platform package (`@tsrs/<platform>`, 24 to 50 MB unpacked, 10 to 20 MB to download), so the experimental checker works from a normal npm or pnpm install with nothing else installed. The checker is still off by default in this release: `--experimental-semantic-types` and the Storybook addon's `semanticTypes: 'on-demand'` stay opt-in, and the installed package is not run otherwise. When the checker is on, the CLI and the addon run that `tsrs` before a `typescript@beta` or `tsrs` the project installed, so a project that relied on its own `typescript@beta` now gets the pinned `tsrs` unless it passes `--tsc-path` (or `tscPath`). Where the package is missing (`cargo install`, or Windows on Arm, where tsrs publishes no binary), the project's checker is used as before, and with none the warning names the package and directory it looked in and what to install.

For library users: `SemanticRequest` and `OnDemandConfig` have a new field, `binary_dir`, the directory of the running binary (or why it is unknown) above which the pinned `@tsrs/<platform>` is looked for; `new` sets it to unknown, so the search starts at the project as before.
