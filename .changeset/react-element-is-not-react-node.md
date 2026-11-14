---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A prop typed `React.ReactElement` or `JSX.Element` is no longer reported as `ReactNode`, which also accepts strings, numbers and `null`: the checker's answer keeps it as the named type `ReactElement<…>` with its arguments, or `JSX.Element`, so Storybook shows the type the prop takes. `JSX.Element` is named through its namespace, so it is not confused with the DOM's `Element`, and in a union such as `Element | React.JSX.Element | null` each member keeps the place it was written in. Only the checker path changes; `--no-checker` still reads both as `ReactNode`. The JSON schema is unchanged.
