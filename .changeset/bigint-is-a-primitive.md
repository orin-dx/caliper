---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

`bigint` is now a type of its own in the canonical JSON: a prop typed `bigint`, or `React.Key` (`string | number | bigint`), has `{ "kind": "bigint" }` where it had a type named `bigint`, whether Caliper read the type itself or the TypeScript checker (`--experimental-semantic-types`) did. The output's `schemaVersion` is 6, so the Vite plugin and the Storybook addon need the same release as the binary, and the parse cache is rebuilt once. The RDT output and the Storybook control are unchanged: the type is still `bigint`, which Storybook has no control for. For library users: `PropType` gains the variant `BigInt`, so an exhaustive `match` on it needs a new arm.
