---
npm/caliper-docgen-vite-plugin: minor
---

- **`injectDocgenInfo`:** assigns each component its react-docgen-typescript docs as `__docgenInfo`, for Storybook 10 and Storybook 11 with `features.docgenServer: false`. A change re-transforms every module whose components it changed, so Storybook's controls follow edits through HMR. Set `typescript.reactDocgen: false` so Storybook's own docgen plugin doesn't overwrite them.
