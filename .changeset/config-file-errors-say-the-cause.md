---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

`docgen.config.ts` problems are reported as what they are. A `--config` path that names no file says `config file <path> does not exist` instead of blaming node. `srcDirs` and `tsconfigPath` in a config resolve against the config file's folder, not the folder the CLI runs from. A default export that is a function or a promise is called and awaited instead of being ignored or failing with a parse error, and one that is not an object says what it is. A config in a package without `"type": "module"` no longer fails with "unknown field `default`".
