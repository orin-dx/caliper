# 0020. Chain dependency layers and load the remaining names after three growths

**Status:** Accepted **Date:** 2026-10-25 **Supersedes:** the 48-growth threshold and full-layer rebuild in [0016](0016-resolve-components-on-request.md)

## Context

ADR 0016 made each growth share the project's indexes, but `dependency_types::load` still rebuilt its own index from every dependency file the session had loaded. After 48 growths it loaded the remaining names in the background to avoid more rebuilds. On shadcn, 50 requests then took 106 ms and held 94 MiB; loading the remaining names after three growths cut the time to about 61 ms but held about 148 MiB. The speed of a static Storybook build matters more than that memory difference, provided answers remain equal to batch extraction.

## Decision

Each growth holds only the declarations and lookup answers it adds, with an `Arc` to the previous layer. Type and import lookups follow the chain. A file parsed earlier as a re-export stub is indexed when a later demand first claims a declaration in it. After eight links, the next growth flattens the chain so lookup depth stays bounded. A lazy session starts loading the rest of the project's dependency names in the background after its third growth.

## Consequences

- At the three-growth threshold, 50 seeded requests are faster than the previous full-layer implementation on every installed corpus: Mantine 42 → 30 ms, Chakra 313 → 234 ms, shadcn 61 → 52 ms, antd 23 → 18 ms, Spectrum 27 → 20 ms. Peak RSS falls or stays close to the previous three-growth run: shadcn 148 → 144 MB. These are best-of-three process measurements with the same corpus and output digest.
- Against the chained implementation at 48 growths, three growths make shadcn's 50 requests 52 instead of 72 ms and its whole-project read 213 instead of 272 ms. The 50-request peak is 144 instead of 94 MB. antd's whole-project read is 56 instead of 126 ms with 81 instead of 84 MB. The other corpora also improve in time; the full table is in `docs/performance.md`.
- All ten workload digests match the prior build. `lazy_parity` matches batch entries and diagnostics over all five installed corpora when components are requested together, forward one at a time, and reverse one at a time. Flattening and a stub claimed later have direct regression tests.
- Watch revisions can keep adding layers. Flattening copies the accumulated layer once per nine growths; a growth that does not flatten processes only newly claimed files.

## Alternative considered

Keep the 48-growth threshold: it saves about 50 MB on shadcn's 50-request sample but makes the whole-project reads slower on all five corpora.
