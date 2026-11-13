---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
---

Merging the checker's answer is faster on a design system: a prop the answer repeats under every component that inherits it, and a file it names again, no longer cost a path allocation and a hash each; a component's props are built into its map in one pass; and the dependency loader keeps its export table without re-hashing it. The output is byte-identical.
