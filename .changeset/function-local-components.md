---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Components declared inside a function body are not documented:** a factory's returned component (`function createLeaf() { const Result = forwardRef(…); return Result; }`) or a local helper exists per call, and no module imports it by that name. Each is now an Info `SKIPPED_CANDIDATE` diagnostic instead of a component; React Spectrum's two `Result` entries and antd's local `PurePanel`, which shadowed the module-level one, are gone.
- **Fewer skip diagnostics:** a PascalCase local that matches no component pattern (`const Comp = asChild ? Slot : "button"`) is no longer reported.
