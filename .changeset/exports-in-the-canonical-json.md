---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

The canonical JSON now says how each component's module exports it: `exports`, a list of `{ name, members }` per component (`default` for the default export, `Card` for `export { _Card as Card }`, and for a sub-component attached to an export the export it hangs from with the property that reaches it, so `Menu.Item = Item` beside `export default Menu` is `default` with `Item`). The session index the Storybook addon reads lists them too. Readers can find a component by file and export instead of by display name, which two components of one file can share. The output's `schemaVersion` is 7, so the Vite plugin and the Storybook addon need the same release as the binary, and `caliper-docgen schema` and the TypeScript declarations list the new field. The RDT and TOON output are unchanged, and the parse cache is not rebuilt. For library users: a `ComponentEntry` read back from the JSON has its `exports`, which it did not before.
