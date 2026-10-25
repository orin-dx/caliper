---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Members of an intersection that reach the same type each keep their props:** `Pick<Partial<T>, K> & Omit<T, K>` (Ark UI's `Optional`) lost its second member as a "circular reference", because the members shared one cycle set. Chakra's Ark-based components (`AccordionRoot`, `SliderRoot`, `ListboxRoot`, `TreeViewRoot`, …) gain 676 props with `--load-dependencies`, 36 false `CIRCULAR_REFERENCE` diagnostics go away, and where an Ark prop overrides a style prop (`CarouselRoot.padding`, `page`) it takes Ark's type, as `Assign<A, B>` does in TypeScript.
- **A destructuring default is the component's default for that prop wherever its type is declared:** `function AlertDialogCancel({ variant = "outline" }: Pick<ComponentProps<typeof Button>, "variant">)` now has `variant: "outline"`; before, only props from an interface or object alias the component names got one (shadcn: 175 defaults).
