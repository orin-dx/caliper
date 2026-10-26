# 0022. Ask a warm checker for the requested component

**Status:** Accepted **Date:** 2026-10-26

## Context

The TypeScript 7.1 backend (ADR 0006, 0007) checks the whole project in one run: 0.5 to 9.4 s and up to 972 MiB on Chakra. A Storybook session asks about one component per story, and confidence scores computed from Caliper's own signals cannot say which components are wrong (`scripts/confidence.py`: at a 10% component budget the warning rule catches 23% of missing props, a risk table 3%, a logistic model 6%). Asking the checker for exactly the requested component needs no prediction.

`tsc --api` builds its program from the root files it is given and loads what they import. A program rooted at one component's file costs far less than the project, and the process stays up between programs. `crates/semantic/examples/ondemand_bench.rs`, 50 random components per corpus, TypeScript 7.1.0-beta, Apple silicon:

| Corpus | First answer | Per component, 1 / 8 / 32 per request | Whole project | Checker peak, one at a time / whole project |
| --- | --: | --: | --: | --: |
| Mantine | 137 ms | 41 / 6.6 / 2.7 ms | 605 ms | 180 / 276 MiB |
| Chakra | 703 ms | 534 / 259 / 108 ms | 9,446 ms | 350 / 972 MiB |
| shadcn | 82 ms | 28 / 8.6 / 3.3 ms | 1,518 ms | 157 / 899 MiB |
| antd | 91 ms | 37 / 6.1 / 8.4 ms | 489 ms | 612 / 436 MiB |
| Spectrum | 131 ms | 21 / 6.5 / 2.2 ms | 618 ms | 218 / 377 MiB |

Of a 30 to 40 ms request, 20 to 33 ms builds the program and 2 to 11 ms answers the queries. Initializing the session and parsing the tsconfig once per process, instead of per request, took 10 ms off. For the same components the checker reports 893 props where the structural pass reports 387 on shadcn, 828 against 732 on antd, and the same count on Mantine, Chakra and Spectrum.

## Decision

A session can have a checker attached (`WatchSession::with_semantic`, a `SemanticProvider` the semantic crate implements). Core owns the trait and the merge; the checker's process, protocol and threads stay in `caliper-docgen-semantic`, so core does no I/O.

- `component(key)` computes the structural answer, asks the checker for that component, and waits up to `semanticWaitMs` (default 150). An answer in time replaces the props, keeping defaults, tags, `parent` and `declarations` from the structural pass, and adds one `Info` provenance diagnostic. A slower checker leaves the structural answer and one `Info` diagnostic saying so; its late answer is kept for the next request.
- Answers are cached in memory by component key and the content hash of its file. An edit is a new question.
- The checker runs on its own thread with its own runtime, so a caller on any thread can bound its wait. `tsc` starts in the background when the session is created. It is replaced after 64 programs (antd reached 612 MiB after 50) and after a crash; three crashes keep it down for the session.
- A missing or unpinned `tsc`, a crash or a timeout yields the structural answer and one `Warning` per session. A request never fails because of the checker.
- The option is `semanticTypes: 'off' | 'on-demand'` on the native session and the Storybook addon, with `tscPath` and `semanticWaitMs`. It is off by default and experimental, as ADR 0007 requires.

## Consequences

- A story gets the checker's props when the checker is warm and answers inside the wait: usually the first story on Mantine, shadcn, antd and Spectrum (82 to 137 ms), and later stories on those at 21 to 41 ms each. That is above the 5 to 20 ms a story can afford, so a story that is not cached takes the structural answer when the checker is busy. Chakra's 534 ms per component never fits the wait; its answer arrives for the next request.
- The gain is where the structural pass is behind: shadcn's props more than double. On Mantine, Chakra and Spectrum the checker adds nothing, and the mode only costs a process.
- Batching a request to 8 files costs 6 to 9 ms per component (Chakra 259), 32 files 2 to 8 ms (Chakra 108). A prefetch of the components a project's stories name would make most requests cache hits; it is not built here.
- The cache is in memory. Reading and writing it under `node_modules/.cache` across runs is the next step, keyed by file hash, tsconfig and lockfile hash, and TypeScript version.
- The pin to `7.1.0-beta` stays until TypeScript 7.1 is stable; a version range and a startup `--version` check replace it then.
- Peak memory is a `tsc` process of 157 to 612 MiB next to the session. The 64-program replacement bounds it; a memory cap by RSS is not built.

## Alternatives considered

- One program over the whole project, started in the background. It costs 0.5 to 9.4 s and 276 to 972 MiB, and a single `tsc` answers requests one at a time, so a story would queue behind it.
- Route only components a score marks risky. The score measured here catches too few of the components that are wrong (#208) to decide which to send.
- Run the checker in the Storybook process. `tsc` is a separate Go binary reached over a pipe; a thread and a runtime of our own keep the session `Send + Sync` without nesting runtimes.
