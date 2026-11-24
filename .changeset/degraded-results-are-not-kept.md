---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

**A checker outage or an unreadable dependency file no longer sticks to a component.** The next request or run asks again and gets the real answer.

- Why it matters: in a watch or Storybook session, one checker timeout or crash used to leave the component on its structural props, with no warning, until a source file changed. A dependency `.d.ts` that could not be read once (too many open files, an install racing the run) used to be saved in the dependency index as "not exported", and every later run trusted that while the package's `package.json` stayed the same.
- What you see now: every answer the unavailable checker affects carries its warning, not just the first one in the session. A run that cannot read a dependency file still warns, and does not write the dependency index.
