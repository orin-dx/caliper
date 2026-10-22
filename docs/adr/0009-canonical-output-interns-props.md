# 0009. Write each distinct prop once in the canonical output

**Status:** Accepted **Date:** 2026-10-21

## Context

Inherited props repeat on every component that inherits them. On Mantine, 19,506 props hold 3,137 distinct values, and the canonical JSON is 19.3 MB. The Vite plugin serves that whole output as its virtual module and re-serves it after every edit, so the browser downloads and parses all of it (34 ms in `JSON.parse` on Mantine) each time. Storybook does not read this format: both its build-time `__docgenInfo` and its Storybook 11 docgen providers take RDT-shaped props, which an integration converts to.

## Decision

The canonical JSON (CLI `--json` and `--format canonical`, NAPI `extractAll` and `initializeSession`, watch updates) writes each distinct prop once, in a top-level `props` table, and components refer to it by id.

- `components[key].props` and `components[key].notableInherited` map a prop name to an id; `props[id]` is the full prop.
- An id is the 64-bit FNV-1a hash of the prop's compact JSON, as 16 hex digits. The same prop gets the same id in every output and update a build writes, so a client can merge tables. Two different props with one id fail serialization instead of one silently replacing the other.
- An `IncrementalUpdate` carries `props` with every entry its `updatedComponents` reference. A client merges it into its table and may drop ids no component references any more.
- Interning is a wire-format concern only. The Rust types keep each `ComponentEntry`'s props inline, and `ComponentEntry` on its own still serializes them inline.
- `--format rdt` and `--format storybook` stay inline: they exist to match other tools' shapes.

## Consequences

- Mantine's output drops from 19.3 MB to 3.6 MB and its parse from 34 ms to 4.5 ms; Spectrum's from 6.9 MB to 4.2 MB.
- Breaking for every reader of the canonical format: a prop is now one lookup away. `packages/napi/index.d.ts` types the table, and readers that want inline props expand them.
- Ids are opaque. They change when a prop's serialized form changes, including across versions, so nothing may store one beyond the output it came from.
- A hash collision fails the whole write. At 100,000 distinct props the chance is about 3 in 10 billion.

## Alternatives considered

- **Positional ids** (index into a table array): shorter, but the same prop moves between revisions, so every watch update would have to resend the table.
- **Interning only types**: the type is half of a repeated prop; descriptions, parents and tags repeat too. Mantine stays at 10.4 MB.
- **Compression only**: gzip already shrinks the output to 0.83 MB in transit, but the reader still parses 19.3 MB.
