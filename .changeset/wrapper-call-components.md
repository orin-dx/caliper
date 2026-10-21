---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Components built by a wrapper call whose props travel as a type argument are now detected, for example `const Button = polymorphicFactory<ButtonFactory>(...)` where `ButtonFactory` is `Factory<{ props: ButtonProps }>`, or `const Item = withContext<HTMLDivElement, ItemProps>("div")`. Detection is by shape, not by library name: a PascalCase binding initialized with a generic call whose first argument is a function, a tag name or a reference, with the props type picked from the type arguments. Element and ref types are skipped, and a guess whose props resolve to nothing is reported as a `SKIPPED_CANDIDATE` note instead of being listed. On the Mantine source this raises component recall against react-docgen-typescript from 29% to 82%. More components can appear in output than before. The on-disk parse cache is rebuilt once (schema 9).
