---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

- **Function types keep every parameter and their return type:** `(value: number) => ReactNode` used to print as `(value: number) => void`, a function with no parameters as `(e: void) => void`, and one with several parameters as opaque text. Parameter and return types are shown as written, names and generics included (`<T>(item: T) => ReactNode`), and a function inside a union or array is parenthesized (`string | ((theme: Theme) => CSSProperties)`).
- **Handler types keep their names:** `onChange?: React.ChangeEventHandler<HTMLInputElement>` used to print as `(e: React.ChangeEventHandler) => void`.
- **Breaking (canonical JSON and Rust API):** `PropType::EventHandler` (`kind: "eventHandler"`) is replaced by `PropType::Function` (`kind: "function"`, with `typeParams`, `params: [{ name, type }]` and `returnType`), and `OpaqueReason::MultiParamFunction` is removed. The parse cache schema moves to 15.
