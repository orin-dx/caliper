---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

Components typed as a function component plus extra static members, like `const Drawer: React.FC<DrawerProps> & { Panel: … } = …` in Ant Design, are now found with their props. No action needed.
