---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Components typed with `React.AriaAttributes`, `React.RefAttributes<T>` or `React.HTMLProps<T>` now list the props React declares for them: the `aria-*` attributes, `ref`, and the element's attributes, following the `htmlAttributes` mode as other React attribute types do. antd's `Statistic` regains its 53 `aria-*` props, `Segmented` its HTML props, and shadcn's react-aria wrappers their `ref`. `key` stays out of the list, including for components that extend `HTMLProps<T>`.
