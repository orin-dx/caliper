---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **`Exclude`, `Extract` and `NonNullable` evaluate over literal types:** `Exclude<"small" | "medium" | "large", "large"> | "default"` is the union `"small" | "medium" | "default"` (an enum in the RDT format, so a select control), as TypeScript evaluates it. They used to print as written.
- **Template literals with number or boolean parts expand:** `` `h${1 | 2 | 3}` `` is `"h1" | "h2" | "h3"`, also inside an alias's arguments.
- **A conditional decided by a member read evaluates:** `Payload["variant"] extends string ? … : string` takes its branch when the member's type is known, including when the payload declares no such member (Mantine's `variant` props now print their variants or `string` instead of the conditional's source).
- **Member reads inside an alias's arguments evaluate,** through mapped types such as csstype's `Fallback<T>`: Chakra's style props print csstype's property types (`ConditionalValue<Property.Filter | … | AnyString>`) instead of `CssProperties["filter"]`.
