---
npm/caliper-docgen-storybook: patch
---

Stories that import a component by its export name now match it when the component's display name is different, like Mantine's `Thumb` whose `displayName` is `@mantine/core/ColorPickerThumb`, so Storybook shows its props instead of falling back. No action needed.
