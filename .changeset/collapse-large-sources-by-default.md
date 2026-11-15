---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Large inherited sources are collapsed by default, wherever they are declared, and the checker's answer is collapsed the same way.

Breaking for the Storybook addon: its default output changes. `collapseInheritedProps` now defaults to `'dependencies'` instead of `'none'`, so an inherited source too large to be an API (a dependency's DOM, ARIA or style-system set of more than 30 props, or any source of more than 200 props) is one inheritance layer of names instead of hundreds of Controls rows. A component's own props and the notable inherited ones are still listed. The props inside such a layer get no Controls row, and a story's `args` naming one shows it as undocumented: that is intended, and a later release adds the props stories and call sites use. Set `collapseInheritedProps: 'none'` to list every inherited prop as before.

For every consumer, the default `dependencies` mode now also collapses a source in the project's own code that declares more than 200 props (a design system's style props, such as Chakra's `SystemProperties` in Chakra's own repository); before, only dependency sources collapsed. With the checker on, the props a collapsed layer names are no longer also listed from the checker's answer: the checker's props follow the structural answer's layers, so both give the same layers.
