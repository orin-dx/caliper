---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Factory payloads with generic props:** a component built from a factory payload whose `props` member has type arguments, such as Mantine's `OverflowList` (`Factory<{ props: OverflowListProps<any>; … }>`), now gets those props. It used to get the payload's own keys (`props`, `ref`, `signature`, `stylesNames`, `vars`) as props. A payload is never used as the props type, and one whose `props` member isn't a named type is reported as a skipped candidate.
