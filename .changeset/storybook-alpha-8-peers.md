---
npm/caliper-docgen-storybook: patch
---

The addon requires Storybook 11.0.0-alpha.8 or later, the first version whose component resolver reads the story file's editor and program instead of its file path. A subcomponent that is a member of a named import, such as `subcomponents: { Option: Select.Option }`, now gets its props from caliper-docgen. If Storybook's resolver throws, the story goes to Storybook's own docgen instead of reporting an error for the subcomponent.
