---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

**A collapsed inheritance layer now records which of its props your project uses.** Each layer gains `used`: the names of its ambient members (declared by `@types/react` or Spectrum's shared DOM types) that consumers pass at a call site or the component's body reads.

- Why it matters: a collapsed layer is no longer a blind spot, because the props your stories and call sites use are named. Which props are listed or collapsed does not change.

**Upgrading**

**Action:** the canonical JSON moves to schema version 8. Read version 8 in any consumer of the JSON; the Vite plugin, the Storybook addon and the scripts in this repository already do.

- Watch and lazy sessions count call sites again for each revision, so an edit to a consumer reaches every component it renders.
- For library users: `InheritedLayer` gains the public field `used`, so it can no longer be built with a struct expression that omits it.
