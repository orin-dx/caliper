---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With the TypeScript checker on (`--experimental-semantic-types`, or `semanticTypes: 'on-demand'`), a component now gets the props of its own export. Before, a component the checker could not find by its name took the props of the module's default export: with `Menu.Item = Item; export default Menu`, `Item` was given `Menu`'s props and reported as resolved by the checker, and with `export { CardImpl as Card }` beside a default export, `CardImpl` was given the default export's props. Now the checker asks for the export the component really has (a renamed export, `default`, or `Menu.Item` through its parent), and a component it cannot find that way keeps the props Caliper read itself, with the reason in a `SEMANTIC_BACKEND` diagnostic. The output's shape is unchanged, and the parse cache is rebuilt once.

For library users: `ComponentMapping` and `ComponentEntry` record how the module exports each component (`exports`, a list of `ComponentExport`), and `ComponentMapping::exported()` replaces the `exported` field. `SemanticTarget` carries the component's output `key` and its `exports` in place of `exported`, and `SemanticComponent` and `SemanticFallback` name the component by that `key` in place of `file_path` and `display_name`; the merge finds the entry by key.
