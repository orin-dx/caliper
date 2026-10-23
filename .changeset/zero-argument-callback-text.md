---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

- A callback prop with no parameters reads `() => void` in `--format rdt`, `--format storybook` and `inspect`, instead of `(e: void) => void`.
- `--format rdt` gives a literal union's type `raw`, its text, beside `name: "enum"` and `value`, as react-docgen-typescript does.
