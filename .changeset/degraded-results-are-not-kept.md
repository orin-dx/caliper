---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

**A checker outage or an unreadable dependency file no longer sticks to a component.** The next request or run asks again and gets the real answer.

- Why it matters: in a watch or Storybook session, one checker timeout, crash or rejected request used to leave the component on its structural props until a source file changed, and a rejected request was also kept on disk as the checker's answer. A dependency `.d.ts` that could not be read once (too many open files, an install racing the run) used to be kept as "not exported", in the session until it restarted and in the dependency index for later runs.
- What you see now: every answer the failed checker affects carries its warning. The component is asked again 30 seconds later or after the next source-file change; until then a request gets the structural props and the warning at once, without waiting on a checker that just failed. A run that cannot read a dependency file still warns; the session's next load and the next run read the file again.

**Breaking:** for Rust users of `caliper-docgen-core`, `SemanticOutcome` has a new field `unavailable` (set only by `SemanticOutcome::unavailable`) and `SemanticFallback` a new field `transient`. A struct literal that names every field no longer compiles: add `unavailable: None` and `transient: false`, or build the outcome with `..Default::default()`.
