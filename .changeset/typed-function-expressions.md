---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Components written as a plain typed arrow or function expression, such as `export const Button = ({ variant }: ButtonProps) => ...`, are now detected. Async and generator functions and functions whose first parameter is not a named type are ignored. On the Ant Design source this raises component recall against react-docgen-typescript from 88% to 96%. More components can appear in output than before. The on-disk parse cache is rebuilt once (schema 8).
