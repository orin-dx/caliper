# caliper-docgen-semantic

## 0.2.0

- Storybook can now ask TypeScript 7.1 for the exact component a story shows, so props that only the type checker can see appear in its controls (shadcn's components gain more than twice as many). It is experimental and off by default: set `semanticTypes: 'on-demand'` in the addon options and install `typescript@beta`. A story waits up to 150 ms for the checker and otherwise shows the usual props, with the checker's answer ready for the next request. Props the checker does not see stay, so `children`, `aria-*` and DOM handlers appear as `htmlAttributes` says, in both modes.

## 0.1.1

- Publish the TypeScript transport and semantic backend crates before the CLI. The CLI's runtime opt-in backend remains available in its crates.io distribution, and every local crate dependency now has a version requirement that Cargo can publish.

