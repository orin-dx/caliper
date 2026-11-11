---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-storybook: patch
---

An anonymous default export is now a component of its own, and a named export of the file's name beside it is another. In `E.tsx`, `export const E = forwardRef(…)` beside `export default forwardRef(…)` used to give two components named `E`, each listing both `E` and `default` as its exports, so a lookup of `E` or of `default` found two and the Storybook addon could not choose. Now the named one is `E` with exports `[E]` and the default one is `E (default)` with exports `[default]`, in whatever order the file writes them. A default export keeps the bare file name when no other component of the file has it, so existing output does not change. A default export a call wraps (`export default React.forwardRef(InternalTable) as T`, `export default memo(forwardRef(Card))`) now lists `default` as an export of the component the call wraps, which it did not before. The Storybook addon no longer reads a story's module source to find that component, and finds it from the exports like any other. The output's shape is unchanged, and the parse cache is rebuilt once.

For library users: `ComponentMapping` has a new field, `bound`, false for a component that is no binding of its name (an anonymous default export or an inline sub-component), and `ComponentMapping::without_binding` sets it.
