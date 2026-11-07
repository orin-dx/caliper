---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A type written as `{ (a: string): number }` is a function type, where it used to come out as an empty object. A type with several call signatures, or a call signature next to properties (react-hook-form's `watch` and `getValues`), is marked as one the tool could not express, with `degradedBy: ["unsupportedExpression"]`, instead of an empty object with no warning. A prop whose index signature key is a template literal such as `` `data-${string}` `` now carries `degradedBy` too.
