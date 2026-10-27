# 0015. A member read through React's types reads React's declaration and what it imports

**Status:** Accepted **Date:** 2026-10-23

## Context

`React.CSSProperties["zIndex"]` was answered by a hand table in `resolver/primitives.rs` before any declaration: every `CSSProperties` member was `string` except eleven `number` ones, so `pointerEvents`, `objectFit` and `resize` lost their options and `width` lost `number`. React declares `CSSProperties extends CSS.Properties<string | number>`, so the answer is in csstype, which React imports. Two rules kept it out of reach: curated mode's React index is read only by HTML-attribute lookups (0013), and the dependency loader skips React's files, so it never followed React's import of csstype (0010).

## Decision

- **Member reads:** an indexed access (`X["key"]`) reads the member from its declaration first (`evaluate::member`: the interface, its ancestors with their type arguments bound, or an alias's body), and the table is the fallback when no declaration answers (0014). In curated mode that read also consults React's index; no other lookup does, so 0013's other guarantees stand. An unimported `React.X` is React's UMD global and resolves to React's declaration file.
- **Loader:** a demand that reaches React's files walks the React declarations it names and loads what they import (csstype's `Properties`), without adding React's own files to the layer. Their imports join the layer's import map so `CSS.Properties` resolves from React's file. An unimported `React.X` in a project or dependency file is a demand on `"react"`.

## Consequences

- With `--load-dependencies`, `CSSProperties` members print csstype's types: `pointerEvents` and `objectFit` become enums, `zIndex` is `"auto" | number | …` with the CSS-wide keywords. On the five benchmark repos 36 more props match react-docgen-typescript's type exactly and none stop matching; without dependencies, 13 more match (literal-union members now flatten into their union, so the union is an enum), and nothing else changes.
- csstype's aliases print expanded, not by name: RDT shows `Width<string | number>` where Caliper shows the union.
- Loading csstype costs Mantine's repo about 10 ms with dependencies (59 → 69 ms): the loader merges its 763 declarations (874 KB) into the layer each run. Without dependencies, timing is unchanged.
- React's files are still never the layer's declarations: a React name still resolves through the React handling (builtins, curated attributes, Full mode's merge).
