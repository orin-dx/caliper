---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
cargo/caliper-docgen-core: minor
---

- **`watch --out` is written at startup,** not only after the first change, so a consumer reading it never waits on an edit.
- **`r` extracts everything again** and rewrites `--out`, for changes the file watcher cannot see such as `tsconfig.json`, `package.json` or `node_modules`. Before, it only refreshed the exit code.
- **An event that changes nothing** (a non-TypeScript or excluded file) no longer rewrites `--out`.
- **Removed components are reported** on change.
- **Output lines in an interactive session start at column 0.** Raw mode had turned off newline translation, so every printed line began where the last one ended.
- `WatchSession::revision()` returns the published revision number.
