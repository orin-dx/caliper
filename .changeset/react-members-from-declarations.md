---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **`CSSProperties["key"]` reads csstype with `--load-dependencies`:** a prop typed `React.CSSProperties["pointerEvents"]` was always `string`, and `["zIndex"]` always `number`. With dependencies loaded, they now have csstype's types, as TypeScript does: `pointerEvents`, `objectFit` and `resize` are enums, `width` is `string | number` plus its keywords. Without dependencies, the previous answers stay.
- **Indexed access reads the declaration first:** `Leaf["value"]` reads through `Leaf`'s ancestors in other files and binds their type arguments, and a member of a React type (`ButtonHTMLAttributes<HTMLButtonElement>["formAction"]`) reads React's declaration in curated mode. These used to print the access unevaluated or the unbound parameter.
- **A literal-union alias inside a union contributes its members:** `Size | "xl"` is one union of literals, so the RDT format shows it as an enum, and a member it repeats is listed once. Printed text is unchanged; in `--json` the alias's members replace a nested `{"kind":"literalUnion"}`.
