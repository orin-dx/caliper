---
npm/caliper-docgen-storybook: patch
---

The Storybook addon now finds the component a story imports by how its module exports it, without reading the module's source: a renamed export (`export { Inner as Card }`), a default export, and a sub-component attached to an export (`Menu.Item` beside `export default Menu`) each resolve to their own component. It no longer takes a file's only component for its default export, or a component whose display name matches the import when the module exports something else under that name; a story whose import names no component the module exports reports that the file has no such component, with the components it does have. A default export a call wraps (`export default React.forwardRef(Inner) as T`) is still read from the source when the module records no export for it.
