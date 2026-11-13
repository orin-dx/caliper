---
cargo/caliper-docgen-semantic: minor
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-storybook: patch
---

The built-in checker is guarded so that it does not take Storybook or the CLI down with it. A panic inside it fails only the request that caused it; the session it unwound through is closed and a fresh one answers the next request (after three, the checker stays off for the session, as for a crashed process). A file nested more than 20,000 brackets deep is not given to it, since `tsrs` overflows its parse stack and aborts the process at a few hundred thousand: the components that read it keep their structural answer with a warning naming the file. Its memory is now bounded: past 768 MiB of its own arenas (which leave out the host's memory) it stops for the session and components keep their structural answer with a warning that says why. It is no longer replaced after 64 programs, which raised its memory instead of lowering it (Chakra over 650 edits: 377 MiB peak against 514).
