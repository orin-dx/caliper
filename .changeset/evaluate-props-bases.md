---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **Props through conditional, indexed and mapped types:** a props type built as `T extends keyof Recipes ? Recipes[T]["__type"] : …` with a known `T`, or as a mapped type over a known type's keys (`{ [K in keyof Variant]?: … }`), now lists its props instead of a warning. Chakra's recipe props (`size`, `variant`, …) now appear: 332 of the 363 react-docgen-typescript reports on Chakra with dependencies loaded, none before.
- **An alias argument that reads a member prints as what it reads:** `ConditionalValue<ButtonVariant["size"]>` prints `ConditionalValue<"sm" | "md" | "lg" | undefined>`, as TypeScript does.
