---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
---

Exclude route handlers that return a constructed response containing JSX from React component extraction. A `GET` handler returning `new ImageResponse(<div />)` no longer appears as a component with `Request` props.
