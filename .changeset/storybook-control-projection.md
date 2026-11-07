---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
---

In Storybook, a prop typed with a single literal (`kind?: 'button'`) now gets a one-option radio control instead of an object control, a prop tagged `@ignore` no longer gets a control, as in Storybook's own docgen, and a boolean in a mixed literal union (`'a' | 1 | true`) stays a boolean option instead of becoming the string "true" (a string containing `|`, `"` or `\` in such a union keeps its options as before). The same props change in `--format rdt` output; the canonical JSON is unchanged. No action needed.
