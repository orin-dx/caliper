---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

**A checker outage or an unreadable dependency file no longer sticks to a component.** The next request or run asks again and gets the real answer.

- Why it matters: in a watch or Storybook session, one checker timeout, crash or rejected request used to leave the component on its structural props until a source file changed, and a rejected request was also kept on disk as the checker's answer. A dependency `.d.ts` that could not be read once (too many open files, an install racing the run) used to be kept as "not exported", in the session until it restarted and in the dependency index for later runs.
- What you see now: every answer the failed checker affects carries its warning. The component is asked again 30 seconds later or after the next source-file change; until then a request gets the structural props and the warning at once, without waiting on a checker that just failed. A run that cannot read a dependency file still warns, and keeps everything it could read: a type a later `export *` target declares, and the React augmentations of the other files. The session's next request and the next run read the file again, and once it reads, the component has its full props and the warning is gone.
- Also covered: a prop that a package's type inherits from another file of the package that could not be read, and a generic component (`as` props) whose instantiation the checker rejected. Each is asked again on the next request instead of being kept without the prop.
- A file that stays unreadable is reported once in each answer, however many requests a session makes.
- A file that stays unreadable (a permission error, a broken install) no longer slows the session down. A component behind it is read again 30 seconds after the failed read, when the file changes or after the next source-file change; until then a request answers from what was read without loading anything. Components that never reach the file keep their cached work.
- A run that cannot read one dependency file still records every other package lookup it completed, so the next run does not walk those again.
- A component the checker was not asked about because the run passed its memory budget is asked again on the next request, once the 30 seconds pass, instead of keeping its structural props until a file changes.
- A checker rejection while looking up a component's module now says what the checker rejected, as other rejections do, instead of "not part of the checker's program".

**Breaking:** for Rust users of `caliper-docgen-core`, `SemanticOutcome` has a new field `unavailable` (set only by `SemanticOutcome::unavailable` for every target and by `SemanticOutcome::leave_unasked` for the keys a run did not ask about; `Unavailable::covers` tells which) and `SemanticFallback` a new field `transient`. A struct literal that names every field no longer compiles: add `unavailable: None` and `transient: false`, or build the outcome with `..Default::default()`. Put two outcomes together with `SemanticOutcome::merge`, which keeps the mark.
