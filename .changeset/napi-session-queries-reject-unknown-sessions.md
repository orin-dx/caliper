---
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

`sessionDocgenInfo`, `sessionComponentAnswers` and `sessionComponentKeys` throw `Unknown session <id>` for a session that was never created or has been closed, as `ensureFresh` does, instead of answering `{}` or `[]` as if the session had no components.
