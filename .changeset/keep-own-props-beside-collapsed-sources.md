---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-storybook: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

**Props you declare stay listed beside a collapsed source.** A component typed `ComponentProps<'div'> & { className: 'mine' }` keeps its own `className` instead of losing it to the collapsed layer.

- Why it matters: your own props are the ones you document, so a collapse must not hide them.
- Limit: a source behind an `Omit` whose key set cannot be resolved is not collapsed, so its props are listed as before.
