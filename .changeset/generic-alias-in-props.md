---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **A generic type alias used as a prop's type takes its arguments.** `size: StyleProp<MantineSize>` now resolves to `MantineSize | MantineSize[]`. Before, it resolved to the alias's own parameter name (`Value | Value[]`), as though `Value` were a type.
- Leaked parameter names in prop types drop from 25,624 to 72 on Mantine, 721 to 111 on react-spectrum, and 23 to 4 on chakra-ui. What remains is mostly generic components' own type parameters, which react-docgen-typescript also leaves unresolved.
- Mantine's `classNames` keys become the real selector names, and MUI's `sx` callbacks name `Theme`.
