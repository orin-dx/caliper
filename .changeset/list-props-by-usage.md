---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Storybook Controls and the CLI now list the DOM, ARIA and event attributes and a dependency's inherited props only where usage shows them: passed at enough of the component's call sites, read or forwarded by its body, or named by its props type (own, picked, required or defaulted). A plain `ComponentProps<'span'>` badge goes from 280 Controls rows to its 2 own props, and a button over Chakra UI's `ButtonProps` from 437 to 9. The rest are not lost: each is a name in its source's inheritance layer (`members`, with the ones the body reads in `used`). Storybook and the CLI now show the same rows. `htmlAttributes` still says which DOM attributes are known, not how many become rows; `collapseInheritedProps: 'none'` lists every resolved prop as before.
