# 0018. Write prop sets and name lists once in the canonical output

**Status:** Accepted **Date:** 2026-10-24

## Context

ADR 0009 interned each distinct prop, but every component still holds a map of its prop names to ids. Chakra with dependencies has 618 components that carry about 836 style props each: 478,559 name-to-id entries that hold 14,755 distinct ones. Those maps are 16.2 MB of the 32.1 MB output. The `omitted` name lists in the inheritance layers are another 7.1 MB (104 distinct lists in 606 layers), and each diagnostic's `affects` list of component keys is 6.7 MB (777 diagnostics, most naming the same 566 components). The Storybook addon reads the session index, which carried the diagnostics: 6.9 MB.

## Decision

Canonical JSON version 2 writes each repeated list once and refers to it by id.

- **Prop sets.** A component's `props` is an array of prop set ids. A prop set is the props one declaring type (`parent`) contributes, as prop name to prop id; props with no declaring type form a set of their own, first. A prop name is in exactly one of a component's sets. Components that inherit a type share its set. The `propSets` table holds each set once.
- **Name lists.** An inheritance layer's `omitted` and a diagnostic's `affects` are ids into a `nameLists` table. A diagnostic with no `affects` has no field, as before.
- **Ids** are 64-bit FNV-1a hashes written as 16 hex digits, like prop ids: of a set's (name, prop id) entries in name order, and of a list's names in order. The same content has the same id in every output and update, so a client merges tables. Two different sets or lists with one id fail serialization.
- **Update and index.** `IncrementalUpdate` carries `propSets` and `nameLists` with every entry its components and diagnostics refer to. The session index (`initializeSessionIndex`) carries `nameLists` for its diagnostics.
- **Reading.** `ExtractionOutput`'s `Deserialize` expands the ids back, so the Rust types stay inline as in ADR 0009. A name in two sets, or an id missing from its table, fails naming it.
- `notableInherited` stays a name-to-prop-id map: it is 159 KB on Chakra. `--format rdt`, `--format storybook` and `--format toon` are unchanged.

`SCHEMA_VERSION` is 2 (ADR 0017).

## Consequences

- Chakra with dependencies: 32.1 MB to 4.3 MB, `JSON.parse` 103 ms to 12 ms, session index 6.9 MB to 1.8 MB. Mantine 3.0 MB to 2.6 MB and shadcn 3.7 MB to 3.6 MB, which share less. Wall time and peak memory don't move: the in-memory model is unchanged.
- Breaking for every reader: a component's props are two lookups away, and a diagnostic's `affects` is one. The Vite plugin and the Storybook addon resolve `affects` when they read; `packages/napi/index.d.ts` types the tables.
- Set boundaries follow `parent`. A type that two components inherit with different `Omit`s yields different sets and shares nothing, which costs size, never correctness.
- Ids are opaque, and change when a set's contents change.

## Alternatives considered

- **One id per whole props map:** Chakra has 232 distinct maps among 618 components, and the variants differ by a few props out of 705. Sets by declaring type reduce 478,559 entries to 14,755.
- **Compressing the output:** gzip would shrink the transfer but not the parse, which is what the addon and the plugin pay on every update.
