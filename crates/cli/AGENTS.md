# crates/cli

Thin command layer — no extraction logic. Each command parses args and delegates to `caliper-docgen-core::pipeline`.

## Layout

```
main.rs              — Cli struct, Command enum, *Args structs, main(), init_tracing()
commands/            — one file per subcommand (extract, watch, inspect, check, completions)
config.rs            — docgen.config.ts loading, maps to PipelineOptions
output.rs            — comfy-table formatting helpers
```

## Rules

Command payloads go to stdout with `println!`; diagnostics, progress and everything on stderr go through `output.rs` or the tracing subscriber — never `eprintln!`. Error display uses `miette`. Progress uses `indicatif`.

Config loading (`config.rs`) runs the user's `docgen.config.ts` via node+tsx. The config file path is passed via env var, not embedded in a shell string — command injection risk otherwise.
