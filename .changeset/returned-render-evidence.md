---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **A function counts as a component only if it returns an element.** Its own `return`s, or its arrow body, must hold JSX or an element factory call. That includes through conditionals, `&&`, arrays and calls such as `items.map(i => <li/>)` or `createPortal(...)`, and a local initialized with JSX and then returned.
  - Returning `null`, `children` or a component call also counts, as does calling a hook.
  - JSX that is built but never returned, JSX only inside a nested callback, or JSX inside a returned object no longer counts. A typed helper that builds a label and returns a string is no longer a component.
- **An element return type must be React's:** `ReactNode`, `ReactElement` or `JSX.Element` imported from React, or not imported at all. The same name imported from another module no longer makes a function a component.
- Across the five calibration corpora this drops one component, antd's `QRcodeStatus`. It returns the result of a render function chosen at runtime, which only its type shows to be an element.
