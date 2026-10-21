---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Component detection now needs evidence that something renders, so typed helpers, selectors, contexts and stores no longer appear as components:

- **Functions:** a PascalCase function with a typed props parameter must contain JSX or an element factory call, call a hook, return `null`, `children` or a component call, or declare an element return type. That covers arrows, function expressions, functions wrapped in a call, and function declarations. `FormatName = (u: User) => string`, `debounce((q: Row) => …)` and generators are now declined with a `SKIPPED_CANDIDATE` note that says why.
- **Generic wrapper calls** (`factory<F>(…)`, `withContext<El, P>("div")`): the first argument must be a function that renders, a component reference, a wrapped component, or a tag name next to an element type or factory payload. `createContext`, `createStore`, `makeLabel<P>("button")` and similar are declined.
- **`.d.ts` declarations:** `declare function Button(props: P): JSX.Element` is now detected.
- **Async server components** are detected.
- **Aliases:** `memo(Card)` no longer hides an exported `Card`. A call that doesn't look like a component wrapper (`getDisplayName(Card)`) is no longer a component.
- **Unresolvable props:** a wrapper component whose props live in an uninstalled package is kept instead of dropped. This restores 81 Chakra components.
