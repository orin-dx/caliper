---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

**A run no longer falls back to structural results because of a memory reading that wrapped below zero.** Output of the same project no longer changes from run to run for this reason.

- Why it matters: in one of 32 default runs on Chakra, the run's memory meter read a few MiB below zero, was taken for about 17 million TiB, and left 620 components with their structural result and a "past this run's budget" warning.
- Nothing to adopt: runs that stayed under the budget are unchanged, and a run past the real budget behaves as before.
