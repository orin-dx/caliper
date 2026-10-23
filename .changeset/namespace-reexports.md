---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Namespace re-exports resolve:** a type reached through a namespace, such as `import { Menu as MenuPrimitive } from './ui'` and then `MenuPrimitive.RootProps`, now resolves when `./ui` re-exports the namespace (`export * as Menu from './menu'`, or `import * as menu from './menu'; export { menu as Menu }`) or declares it (`export namespace Menu`).
