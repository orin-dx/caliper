---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-vite-plugin: patch
npm/caliper-docgen-storybook: patch
---

- **A union a dependency declares as a tuple is expanded:** `type Align = (typeof ALIGN_OPTIONS)[number]` over `declare const ALIGN_OPTIONS: readonly ["start", "center", "end"]` is `"start" | "center" | "end"`, read from the file that wrote it, as TypeScript reads `typeof X`. Radix declares `Align` and `Side` this way, so shadcn's Radix-based popover, tooltip, hover card, dropdown menu and select now show their `align` and `side` literals instead of the bare names.
- **A type that comes from a call says so:** the help of "typeof 'X' could not statically evaluate" names the cause (a value built by a call, such as a zod schema behind `z.infer`) and the remedies.
