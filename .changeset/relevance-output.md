---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

The canonical JSON moves to schema version 8: a collapsed inheritance layer gains `used`, the names of its ambient members (declared by `@types/react` or Spectrum's shared DOM types) that consumers pass at the call-site floor or the component's body reads (ADR 0028). Which props are listed or collapsed does not change. `InheritedLayer` gains the public field `used`, so it can no longer be built with a struct expression that omits it, and `GlobalSourceData` a crate-internal field (each file's JSX outside component bodies). Watch and lazy sessions count call sites again for each revision, so an edit to a consumer reaches every component it renders. The Vite plugin, the Storybook addon and the scripts read version 8 only. The parse cache moves to version 42.
