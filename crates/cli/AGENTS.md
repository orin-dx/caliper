# crates/cli

Thin command layer — no extraction logic. Each command parses args and delegates to `oxc-react-docgen-core::pipeline`.

## Layout

```
main.rs              — Cli struct, Command enum, *Args structs, main(), init_tracing()
commands/            — one file per subcommand (extract, watch, inspect, check, completions)
config.rs            — docgen.config.ts loading, maps to PipelineOptions
output.rs            — comfy-table formatting helpers
```

## Rules

All user-facing output goes through `output.rs` or the tracing subscriber — never `println!` in command handlers. Error display uses `miette`. Progress uses `indicatif`.

Config loading (`config.rs`) runs the user's `docgen.config.ts` via node+tsx. The config file path is passed via env var, not embedded in a shell string — command injection risk otherwise.
