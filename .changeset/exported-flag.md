---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

- **Every component says whether its module exports it:** `ComponentEntry` gains `exported`. A component counts as exported when it is an exported declaration, a local named by `export { X as Y }` or `export default X`, an anonymous default export, or a sub-component attached to an exported component (`Parent.Sub = Sub`, `Object.assign(Parent, { Sub })`, `const Parent = Object.assign(Root, { Sub })`).
- **`--format rdt` and `--format storybook` omit unexported components by default**, as react-docgen-typescript documents only exports. Pass `--include-unexported` to keep them. Canonical output keeps every component. Measured: Mantine 7 of 346 unexported, Chakra 2 of 600, shadcn 99 of 1,884, Ant Design 42 of 253, React Spectrum 194 of 1,359.
- The DTS cache schema moves to 15, so existing caches are rebuilt once.
