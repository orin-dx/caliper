---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

- **Aliases over conditional or mapped types keep their names:** a prop typed `StyleProp<MantineSpacing>` or `MantineBreakpoint` used to print the alias's expansion, including conditional-type source such as `MantineThemeSizesOverride extends { ... } ? CustomSpacing : …`. It now prints the alias name with its arguments as written, the way TypeScript does. Aliases to literal unions and objects still expand, so enum controls are unchanged.
