---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Inherited React event handlers take their element:** with `--html-attributes full`, a component extending `React.SVGProps<SVGSVGElement>` or `ButtonHTMLAttributes<HTMLButtonElement>` listed `onClick` as `(event: MouseEvent<T>) => void`, with React's type parameter unbound. It is now `(event: MouseEvent<SVGSVGElement>) => void`, as TypeScript reads it.
