---
npm/caliper-docgen-storybook: minor
---

- **New package `caliper-docgen-storybook`:** a Storybook 11 addon whose docgen provider answers argTypes from a caliper-docgen session. It resolves `meta.component` and `meta.subcomponents`, converts with `@storybook/react`'s own converter, applies saved edits before each answer, and hands a component it can't find, or one whose extraction reported a warning or error, to React's docgen. On a four-component example its first answer took 43 ms against React's 504 ms, and later answers 0.4 ms against 0.7–1.5 ms.
