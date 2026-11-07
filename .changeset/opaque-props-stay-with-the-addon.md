---
npm/caliper-docgen-storybook: patch
---

The README now says a prop the extractor reports only as opaque (an info diagnostic) does not hand the component to Storybook's docgen. Measured on Mantine, Chakra, shadcn and antd, counting it would hand on 51% to 91% of components, so the rule is unchanged. No action needed.
