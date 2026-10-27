# 0021. Publish the semantic backend crates with the CLI

**Status:** Accepted **Date:** 2026-10-25

## Context

The 0.6.0 release published `caliper-docgen-core`, then Cargo rejected `caliper-docgen` because its `caliper-docgen-semantic` dependency had only a local path. Both the semantic backend and its `caliper-docgen-tsc-client` dependency were marked `publish = false`. ADR 0007 requires the backend in every CLI build, with activation controlled at runtime, so the published CLI also needs both crates from crates.io.

## Decision

Publish `caliper-docgen-tsc-client` and `caliper-docgen-semantic` as separate crates. Every path dependency in their chain and in `caliper-docgen` has a version requirement. Callisto includes the two crates as crates.io release packages so it can publish them before the CLI. The backend remains off at runtime unless requested.

## Consequences

- Cargo can package the CLI from registry dependencies, and Callisto can order the transport, backend and CLI publications by dependency.
- The two backend crates become public distributions whose versions and API compatibility need release maintenance.
- The failed 0.6.0 release commit cannot be repaired by rerunning its immutable source; a new release must include these manifest changes.

## Alternatives considered

- Move the backend and transport code into the CLI crate. That removes the registry dependencies but collapses two existing crate boundaries and makes the CLI own the process transport.
- Exclude the backend from the published CLI. That would make the binary depend on how it was built, reversing ADR 0007's always-compiled contract.
