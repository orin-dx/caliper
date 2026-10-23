# 0011. Find dependency declarations through a per-package export index, not the parse cache

**Status:** Accepted **Date:** 2026-10-23

## Context

Following a name from a package's entry point to the file that declares it walks re-export chains: chakra's `@ark-ui/react` is 1,434 mostly one-line `.d.mts` re-export files, and shadcn's icon packages re-export 1,500+ names each. Each hop is a file open and a resolver call, and opening files is the cost (0010). The msgpack `DtsCache` (0003, 0008) stores whole parsed files; measured on the dependency closures, a warm run was no faster than re-parsing in 6 of 7 cases (antd 80 ms warm against 45 ms with no cache), loading the cache alone cost 11–40 ms, and its 5,000-entry cap thrashed on shadcn's 6,217 files.

## Decision

The loader keeps a per-package export index next to the parse cache, and does not put dependency files in the `DtsCache`.

- **Contents:** for each package entry module the loader looked up, its re-export graph flattened to exported name → (declaring file, local name), including `export *`, `export { a as b } from`, `export * as ns` and `import x; export { x }`; a name exported across a package boundary points into the other package's index. Modules whose exports are all their own declarations are not stored.
- **Keying and validation:** one file per package, keyed by the package's real path and a hash of the resolver configuration (tsconfig path and mtime, conditions). It is reused only while the package's `package.json` has the same mtime and size, and discarded with the rest of the cache when the build changes (0008).
- **Declaring files are parsed each run.** Parsing is cheap (418 MB/s); with the index, a warm run opens only the files that declare a loaded name.

## Consequences

- Warm runs open no re-export stub: resolver calls inside dependencies fell from 4,842 to 603 on shadcn and from 2,171 to 572 on chakra in the prototype, and loading cost 2–29 ms on the five projects.
- A package edited in place without touching `package.json` (patch-package) keeps a stale index until the cache is cleared.
- The index is written after loading; a cold run pays 10–55 ms more than a warm one.
