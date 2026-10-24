---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
---

**Experimental `--experimental-semantic-types`:** `extract` asks TypeScript 7.1's `tsc --api` (`typescript@beta`, found at `node_modules/typescript/bin/tsc` or `--tsc-path`) for each exported function component's props and merges the checker's props, types and required flags over the structural result. Config keys `experimentalSemanticTypes` and `tscPath`. Off by default, and without the flag the output is byte-identical.

The structural result stands wherever the checker cannot answer: a missing, unpinned, crashed or timed-out `tsc` adds one `SEMANTIC_BACKEND` warning; a class, overloaded, generic or union-props component, or one the module does not export, keeps its props and is listed by an `Info` diagnostic whose `affects` names it. The components the checker resolved are listed the same way. Checker types map onto the existing `PropType`; what it cannot represent (optional and rest parameters, index signatures, `readonly`) is not yet carried.
