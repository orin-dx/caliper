---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

**A checker outage or an unreadable dependency file no longer sticks to a component.** The next request or run asks again and gets the real answer.

- Why it matters: in a watch or Storybook session, one checker timeout, crash or rejected request left the component on its structural props until a source file changed.
- Why it matters: a rejected checker request was saved on disk as the checker's answer for later runs.
- Why it matters: a dependency `.d.ts` that could not be read once (too many open files, an install racing the run) counted as "not exported", for the rest of the session and for later runs.

**What you see now**

- Every answer a checker failure affects carries a warning that says so.
- A component the checker failed on now gets a warning, not an info note, so Storybook's `fallback: 'degraded'` takes the configured fallback for it.
- The checker is asked about the component again 30 seconds after it failed, or after the next source-file change.
- Until then, a request gets the structural props and the warning at once, without waiting on the checker.
- A component the checker skipped because it passed its memory budget is asked again the same way.
- A generic component (`as` props) whose request the checker rejected is asked again instead of being kept without its props.
- A rejected lookup of a component's module says what the checker rejected, instead of "not part of the checker's program".
- A dependency file that cannot be read is reported once in each answer it affects.
- The answer keeps every type the other files gave, including one a later `export *` target declares and a package type's own props.
- Once the file reads, the component has its full props, including a prop a package type inherits from that file, and the warning is gone.
- A file that stays unreadable does not slow the session down, and components that never reach it are not affected.
- A run that cannot read one dependency file still remembers every other package lookup, so the next run is as fast as before.

**Limits**

- A dependency file fixed on disk (a `chmod`, a finished install) is picked up 30 seconds after the failed read or at the next source-file change, not at once.

**Breaking:** for Rust users of `caliper-docgen-core`, `SemanticOutcome` has a new field `unavailable` (set only by `SemanticOutcome::unavailable` for every target and by `SemanticOutcome::leave_unasked` for the keys a run did not ask about; `Unavailable::covers` tells which) and `SemanticFallback` a new field `transient`. A struct literal that names every field no longer compiles: add `unavailable: None` and `transient: false`, or build the outcome with `..Default::default()`. Put two outcomes together with `SemanticOutcome::merge`, which keeps the mark.
