---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With the experimental TypeScript checker on, a prop whose name starts with two underscores keeps the name the source writes. Mantine's `__size`, `__vars` and `__clearable` used to come back as `___size`, `___vars` and `___clearable` (the checker keeps such names with a third underscore), so those props appeared under a name nobody wrote.
