---
cargo/caliper-docgen: minor
npm/caliper-docgen: minor
---

`extract --experimental-semantic-types` takes `--semantic-timeout-ms`: how long the TypeScript checker may take to start or to answer any one request before the run uses the structural result, with one warning that says it timed out. It defaults to 10 seconds, down from a fixed 60, so a checker that hangs is given up on sooner; it does not bound the whole run, which on a large project can take longer while the checker keeps answering. Raise it if your checker is slow to build a large program.
