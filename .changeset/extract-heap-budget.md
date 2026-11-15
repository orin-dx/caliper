---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

`extract` with the checker on now stops asking the checker once its heap passes a budget: 60% of the memory the process may use (the machine's, or a lower cgroup limit on Linux), at most 8 GiB. The components not yet answered keep their structural result, and one `SEMANTIC_BACKEND` warning says how many and why. A project whose own source declares a very large style system (Chakra's `packages/react/src`) grew to 42 GB before; with the budget it finishes at the budget. Which components the checker answered before the budget was reached depends on timing, so such an output can differ between runs; every corpus we measure stays at about 1 GiB, far under the budget, and its output is unchanged.
