---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

**`extract` with the checker on now stays within a memory budget.** It stops asking the checker once its heap passes 60% of the memory the process may use, at most 8 GiB.

- Why it matters: a project whose own source declares a very large style system (Chakra's `packages/react/src`) grew to 42 GB before and now finishes at the budget.
- What you see: components not yet answered keep their structural result, and one `SEMANTIC_BACKEND` warning says how many and why.
- On Linux the budget follows a lower cgroup limit, v1 (including comounted mounts) or v2.
- Limit: which components the checker answered before the budget was reached depends on timing, so such an output can differ between runs.
- Limit: projects we measure stay near 1 GiB, far under the budget, and their output is unchanged.
