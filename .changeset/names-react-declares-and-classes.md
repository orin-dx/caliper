---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Names that were reported unresolvable and are not:** each is now the type it names, without a warning, which keeps the component out of the Storybook addon's fallback.
  - A type React declares that no builtin list names (`UIEventHandler`, `Key`, `AriaRole`, `ReactDOMAttributes` imported from `DOMAttributes`), read from React's own declarations.
  - A generic component's own type parameters (`function ActionBar<T extends object>(props: ActionBarProps<T>)`), registered as the placeholders they are.
  - A class used as a type (`value: CalendarDate`), recorded as a declaration with no members.
  - ES2015 globals: `Iterable`, `Set`, `Map` and `Promise`, read from TypeScript's `lib.es2015.*` files when installed.
- Components with a warning, with `--load-dependencies`: Spectrum 29% → 0.4%, antd 27% → 9%.
- The parse cache schema moves to 22 (component mappings record their type parameters).
