---
npm/caliper-docgen-storybook: patch
---

The addon's option keys are listed once, in a list the compiler checks against `CaliperStorybookOptions`, so a new option cannot be left out of what the addon passes to the docgen worker.
