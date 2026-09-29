# 0007. A session owns its child's I/O and closes once, with a reason

**Status:** Accepted **Date:** 2026-09-28

## Context

`crates/tsc-client` is the Rust client for `tsc --api` (ADR 0006). Its first cut let any caller's `request()` future write to the child's stdin under a shared mutex, and nothing recorded that a session had ended. An audit against the Lifecycle gate in `docs/type-checker-integration.md` reproduced six failures from that one cause:

- Dropping a request future mid-write left a partial frame on the pipe. Every later request on every `Client` clone timed out, and `is_alive()` stayed `true`.
- A child that stopped reading stdin blocked a write past its deadline and queued every other caller behind the mutex.
- A request made after the reader exited, or in flight during `shutdown()`, waited out its whole deadline and reported `Timeout`.
- A fatal frame error from a child that stayed alive was never restarted, because the supervisor only matched `Crashed | Write`.
- Stdin closed only when every `Client` clone dropped, so shutdown always spent the 2 s grace period and then sent SIGKILL.

Patching each one leaves the cause in place: no one owned the pipe, and no state said the session was over.

## Decision

A session owns its I/O and ends exactly once.

- **One writer.** A writer task alone holds `ChildStdin`. Callers send whole frames over a bounded channel (`WRITE_QUEUE_FRAMES = 16`), so a dropped request future has enqueued a full frame or nothing.
- **One close.** A set-once closed state carries the reason. Reader exit, a write failure and `Supervisor::shutdown` all set it. Setting it fails every pending waiter and stops the writer, which drops stdin even while `Client` clones are alive.
- **One deadline.** `request()` puts enqueue and response under a single deadline and fails at once when the session is closed.
- **Restarts follow state.** The supervisor decides restarts from the closed state only. It never matches error variants or polls task handles. Shutdown closes the session first, then reaps the child and returns its exit status.

## Consequences

- A task and a channel replace a mutex. That is the point: the mutex version is what the audit broke.
- The queue is bounded in frames, not bytes. Sixteen 8 MiB frames can hold 128 MiB.
- Any close reason triggers a restart. A child that always sends a bad header burns the three-restart budget in three calls, and the storm cap then stops it.
- A `Client` clone from before a restart fails at once with its own session's reason, not a generic error.
- Each task holds a guard that closes the session on drop, so a panicking task cannot leave a session open.
- A cancelled `Supervisor::request` can burn one restart attempt.
- Enforced by tests: `dropping_a_request_future_mid_write_never_wedges_the_session`, `a_child_that_stops_reading_stdin_cannot_hold_a_request_past_its_deadline`, `shutdown_wakes_requests_blocked_on_a_full_write_queue`, `shutdown_fails_in_flight_requests_on_clones_with_session_closed` and `a_fatal_frame_from_a_live_child_is_restarted_not_timed_out`. `ChildStdin` is moved into the writer task and sits in no struct field, so no other code can write to the pipe.

## Alternatives considered

- **Patch each instance** (a liveness check per call, a stdin timeout, abort the reader after draining). Each patch leaves the shared cause, and the next instance is one edit away.
- **Keep the mutex, add a write timeout.** A timed-out `write_all` still leaves a partial frame that desyncs tsc's decoder.
- **Unbounded queue.** Memory grows without limit against a child that stops reading.
