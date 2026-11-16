---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
---

The structural answer (`--no-checker`, and what the checker lane keeps from it) now reads more prop types the way the checker does:

- `React.ReactElement` and `JSX.Element` stay named `ReactElement` / `JSX.Element` instead of widening to `reactNode`.
