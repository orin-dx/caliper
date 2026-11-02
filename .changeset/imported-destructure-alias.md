---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

A component re-bound from an import is found in two more forms, `const { Option } = Imported` and `const Chart = Imported`, and its props are the imported component's, also when the package declares it as a `React.MemoExoticComponent` or behind a re-exported default. On shadcn/ui that adds `ChartLegend` (four registries) and `Form`; on Ant Design, `Option`. An alias of a component the project already declares under that name is not reported twice.
