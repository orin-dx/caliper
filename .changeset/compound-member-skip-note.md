---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

`Root.Item = memo(...)` and `Root.Item = forwardRef(...)` now leave a skip note saying the sub-component is not read, instead of vanishing from the output without a word. Other assigned calls, such as `Root.Context = createContext(...)`, stay silent because they are not components.
