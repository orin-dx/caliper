---
cargo/caliper-docgen: minor
npm/caliper-docgen: minor
---

- **Breaking:** `--format storybook` is removed. Storybook reads react-docgen-typescript's shape, so use `--format rdt`, or the live integrations: `caliper-docgen-storybook` on Storybook 11, and the Vite plugin's `injectDocgenInfo` without a docgen server.
