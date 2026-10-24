---
npm/caliper-docgen-storybook: patch
---

- **A story's component is found when its file holds several:** `import Card from './Card'` now reaches the component the module exports as `default`, `export { Inner as Card }` reaches `Inner`, and a default import falls back to the name the story gave it. On ant design 83 sampled stories, the addon answers 77 where it answered 58 (no story finds "no component in session" any more; five stay with Storybook for incomplete props).
- **`component: Select.Option` works with a named import:** Storybook's resolver reads a member only of `import * as`; the addon now finds the `Select.Option` component.
- **A story the session can't match says why:** the debug log lists the components the file does have.
