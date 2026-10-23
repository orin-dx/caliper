---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **A JSDoc comment documents only the declaration right after it:** a comment used to attach to whatever started within a few lines after it, so the first prop of `function Badge(props: { tone: string })` took the component's description, an inline object field took its prop's description or `@deprecated` tag, and a comment above a `const` or a license header could become a component's description. Only modifiers, keywords, decorators and other comments may now stand between a comment and what it documents, as in TypeScript.
