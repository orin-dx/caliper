---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **An alias over an indexed access that can't be evaluated keeps its name:** with `--load-dependencies`, `z.infer<typeof schema>` printed as `typeof schema["_output"]`. Like an alias over a conditional or mapped type, it now shows as `z.infer<typeof schema>`, and project aliases such as `type Side = (typeof SIDE_OPTIONS)[number]` show as `Side` instead of their source. An indexed access on a declared type passed as an argument (`z.infer<Parsed>`) now evaluates.
- **`typeof X` keeps its `typeof`:** a value's type that can't be evaluated printed as the bare value name (`ReturnType<useTree>`, `JSON.parse`); it now prints as `ReturnType<typeof useTree>` and `typeof JSON.parse`, as TypeScript does.
