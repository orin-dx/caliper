---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

**`ReactElement` and `JSX.Element` props keep their names.** The structural answer (`--no-checker`, and what the checker lane keeps from it) shows them as the checker does, instead of widening them to `reactNode`.

- Limit: structural `Partial`, `Required`, `Pick` and `Omit` are not expanded; the checker answers them.
