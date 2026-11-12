---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-storybook: patch
---

With the TypeScript checker on (`semanticTypes: 'on-demand'`, `--experimental-semantic-types`), a component the checker answers for no longer keeps the warnings Caliper's own pass reported about what that answer replaces, such as "Cannot find the component 'Button'" or a prop type it could not resolve. In Storybook, with the default `fallback: 'degraded'`, such a component (Chakra's `Button` among them) now shows the checker's props instead of being handed to React's docgen. Warnings about what the checker could not read either, and the checker's own (it stopped, it is unavailable), stay and still hand the component on. A warning shared by several components now lists only the ones the checker did not answer. For library users: `DiagnosticCode::superseded_by_checker` says, for each code, whether a checker answer supersedes it (the new `Superseded` enum).
