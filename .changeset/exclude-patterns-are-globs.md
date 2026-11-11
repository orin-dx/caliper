---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-napi: patch
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

`exclude` patterns are now globs, as the Storybook README always said, matched against each file's path inside its source directory. `*.fixture.tsx`, `legacy/` and `**/internal/**` work; before, a pattern excluded any file whose path merely contained that text, so `Button` also dropped `ButtonGroup.tsx`. Now `Button` excludes only a file or folder named exactly `Button`. A pattern that is not a valid glob is reported and excludes nothing, and an empty pattern excludes nothing instead of every file. Patterns follow gitignore: `['*', '!*/', '!*.tsx']` keeps every `.tsx` file in every folder, including the folders themselves in watch mode (a file created later is noticed), while a `!` pattern cannot bring back anything inside a folder an earlier pattern excluded. The error for a bad `docgen.config.ts` now lists exactly the options the config accepts.
