#![recursion_limit = "256"]
// stdout carries the command payload (JSON, tables); stderr and diagnostics still go through output.rs.
#![allow(clippy::print_stdout)]

mod commands;
mod config;
mod output;

use clap::{Parser, Subcommand};
use miette::Result;

use commands::check::cmd_check;
use commands::completions::cmd_completions;
use commands::extract::cmd_extract;
use commands::inspect::cmd_inspect;
use commands::schema::cmd_schema;
use commands::watch::cmd_watch;

// Resolution allocates from every rayon worker at once, and the system allocator serializes them (macOS most of all).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(
    name = "caliper-docgen",
    about = "Fast React prop extraction powered by OXC",
    version,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Verbose output (repeat for more: -v, -vv)
    #[arg(global = true, long, short, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress all non-error output
    #[arg(global = true, long, short)]
    quiet: bool,

    /// Path to docgen.config.ts
    #[arg(global = true, long)]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Extract prop types and write to stdout or --out file
    Extract(ExtractArgs),

    /// Watch for changes and re-extract
    Watch(WatchArgs),

    /// Validate extraction — exits 2 if any errors, or 1 if --strict and there are warnings. For CI.
    Check(CheckArgs),

    /// Show resolved props for a single component (debugging tool)
    Inspect(InspectArgs),

    /// Export JSON Schema for docgen output format
    Schema,

    /// Generate shell completions
    Completions(CompletionsArgs),
}

#[derive(clap::Args)]
pub struct ExtractArgs {
    /// Source directories to scan [default: ./src]
    #[arg(long, short, value_delimiter = ',')]
    pub src: Vec<String>,

    /// Output file [default: stdout]
    #[arg(long, short)]
    pub out: Option<String>,

    /// Output format
    #[arg(long, short, default_value = "canonical")]
    #[arg(value_enum)]
    pub format: OutputFormat,

    /// Cache directory
    #[arg(long)]
    pub cache_dir: Option<String>,

    /// Neither read nor write the on-disk caches (the declaration parse cache and the dependency index); the output is
    /// the same, a run just parses everything
    #[arg(long)]
    pub no_cache: bool,

    /// How much of an inherited HTML element's attributes to expose
    #[arg(long, value_enum)]
    pub html_attributes: Option<HtmlAttributeModeArg>,

    /// Extra type names to treat as recognized/builtin (no "unknown type" warning) —
    /// e.g. a library-specific type this tool doesn't already know
    #[arg(long, value_delimiter = ',')]
    pub extra_builtins: Vec<String>,

    /// Keep components their module doesn't export in `--format rdt`, which omits them by default as
    /// react-docgen-typescript does. Canonical output always keeps them, marked `exported: false`.
    #[arg(long)]
    pub include_unexported: bool,

    /// Machine-readable JSON output: the canonical schema, compact, always to
    /// stdout, ignoring --out/--format (which govern the human/RDT/TOON
    /// paths below), and suppresses the human-readable summary/diagnostics.
    #[arg(long, short = 'j')]
    pub json: bool,
    /// Load the type declarations props reference from installed packages (on by default; this flag turns it back on
    /// over a config that sets `loadDependencies: false`)
    #[arg(long)]
    pub load_dependencies: bool,
    /// Do not load the type declarations props reference from installed packages
    #[arg(long, conflicts_with = "load_dependencies")]
    pub no_load_dependencies: bool,

    /// Which inherited sources of more than 30 props to list as one inherited layer instead of prop by prop
    #[arg(long, value_enum)]
    pub collapse_inherited_props: Option<CollapseInheritedPropsArg>,

    /// Experimental: ask TypeScript 7.1's `tsc --api` for each component's props and merge them over the structural
    /// result, which stands wherever the checker cannot answer (off by default)
    #[arg(long)]
    pub experimental_semantic_types: bool,

    /// The `tsc` binary `--experimental-semantic-types` runs, instead of `node_modules/typescript/bin/tsc`
    #[arg(long, requires = "experimental_semantic_types")]
    pub tsc_path: Option<String>,
}

#[derive(clap::ValueEnum, Clone)]
pub enum OutputFormat {
    /// caliper-docgen canonical schema (richest)
    Canonical,
    /// react-docgen-typescript compatible output
    Rdt,
    /// Token-Optimized Object Notation (compact agent format)
    Toon,
}

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum HtmlAttributeModeArg {
    /// ~15-20 curated, commonly-documented attributes per element (default)
    Curated,
    /// All of `@types/react`'s real attributes for the element (matches RDT)
    Full,
    /// No inherited HTML attributes at all — own props only
    None,
}

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum CollapseInheritedPropsArg {
    /// Sources from loaded dependencies (default)
    Dependencies,
    /// Dependency and project sources
    All,
    /// List every prop
    None,
}

impl From<CollapseInheritedPropsArg> for caliper_docgen_core::pipeline::CollapseInheritedProps {
    fn from(arg: CollapseInheritedPropsArg) -> Self {
        match arg {
            CollapseInheritedPropsArg::Dependencies => Self::Dependencies,
            CollapseInheritedPropsArg::All => Self::All,
            CollapseInheritedPropsArg::None => Self::None,
        }
    }
}

#[derive(clap::Args)]
pub struct InspectArgs {
    /// Component name to inspect
    pub component: String,

    /// Source directories to scan [default: ./src]
    #[arg(long, short, value_delimiter = ',')]
    pub src: Vec<String>,
    /// Load the type declarations props reference from installed packages (on by default; this flag turns it back on
    /// over a config that sets `loadDependencies: false`)
    #[arg(long)]
    pub load_dependencies: bool,
    /// Do not load the type declarations props reference from installed packages
    #[arg(long, conflicts_with = "load_dependencies")]
    pub no_load_dependencies: bool,

    /// Which inherited sources of more than 30 props to list as one inherited layer instead of prop by prop
    #[arg(long, value_enum)]
    pub collapse_inherited_props: Option<CollapseInheritedPropsArg>,
}

#[derive(clap::Args)]
pub struct WatchArgs {
    /// Source directories to watch [default: ./src]
    #[arg(long, short, value_delimiter = ',')]
    pub src: Vec<String>,

    /// Output file to write on each change
    #[arg(long, short)]
    pub out: Option<String>,
    /// Load the type declarations props reference from installed packages (on by default; this flag turns it back on
    /// over a config that sets `loadDependencies: false`)
    #[arg(long)]
    pub load_dependencies: bool,
    /// Do not load the type declarations props reference from installed packages
    #[arg(long, conflicts_with = "load_dependencies")]
    pub no_load_dependencies: bool,

    /// Which inherited sources of more than 30 props to list as one inherited layer instead of prop by prop
    #[arg(long, value_enum)]
    pub collapse_inherited_props: Option<CollapseInheritedPropsArg>,
}

#[derive(clap::Args)]
pub struct CheckArgs {
    /// Source directories to scan [default: ./src]
    #[arg(long, short, value_delimiter = ',')]
    pub src: Vec<String>,

    /// Fail on warnings in addition to errors (exits 1, distinct from the exit-2 error path)
    #[arg(long)]
    pub strict: bool,

    /// Machine-readable JSON diagnostics instead of the human-readable table
    #[arg(long, short = 'j')]
    pub json: bool,
    /// Load the type declarations props reference from installed packages (on by default; this flag turns it back on
    /// over a config that sets `loadDependencies: false`)
    #[arg(long)]
    pub load_dependencies: bool,
    /// Do not load the type declarations props reference from installed packages
    #[arg(long, conflicts_with = "load_dependencies")]
    pub no_load_dependencies: bool,

    /// Which inherited sources of more than 30 props to list as one inherited layer instead of prop by prop
    #[arg(long, value_enum)]
    pub collapse_inherited_props: Option<CollapseInheritedPropsArg>,
}

#[derive(clap::Args)]
pub struct CompletionsArgs {
    pub shell: clap_complete::Shell,
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let cli = Cli::parse();

    init_tracing(cli.verbose);

    let exit_code = match cli.command {
        Command::Extract(args) => cmd_extract(args, cli.quiet, cli.config.as_deref())?,
        Command::Watch(args) => cmd_watch(args, cli.quiet, cli.config.as_deref())?,
        Command::Check(args) => cmd_check(args, cli.quiet, cli.config.as_deref())?,
        Command::Inspect(args) => cmd_inspect(args, cli.config.as_deref())?,
        Command::Schema => {
            cmd_schema()?;
            0
        }
        Command::Completions(args) => {
            cmd_completions(args)?;
            0
        }
    };

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// The tracing filter for `-v` given `verbose` times; none means no subscriber at all.
fn log_filter(verbose: u8) -> Option<&'static str> {
    match verbose {
        0 => None,
        1 => Some("info"),
        2 => Some("debug"),
        _ => Some("trace"),
    }
}

fn init_tracing(verbose: u8) {
    let Some(filter) = log_filter(verbose) else { return };
    use tracing_subscriber::{fmt, EnvFilter};
    // stderr, never stdout: command payloads (JSON, TOON) own stdout, and a tracing line there corrupts them.
    fmt().with_env_filter(EnvFilter::new(filter)).with_writer(std::io::stderr).init();
}

#[cfg(test)]
mod tests {
    use super::log_filter;

    #[test]
    fn each_extra_v_raises_the_log_level_and_zero_installs_no_subscriber() {
        let filters: Vec<Option<&str>> = [0, 1, 2, 3, u8::MAX].into_iter().map(log_filter).collect();
        assert_eq!(filters, [None, Some("info"), Some("debug"), Some("trace"), Some("trace")]);
    }

    // ── SPEC-CLI-001a AC-017: command handlers return their exit code from
    // cmd_extract/cmd_check/etc. rather than calling std::process::exit
    // directly, so main() is the sole place that terminates the process —
    // this makes the exit decision testable. Nothing previously enforced
    // this beyond visual inspection; this test greps the actual committed
    // source for every `std::process::exit(` call site and pins the
    // allow-list to exactly the two known, documented exceptions.

    #[test]
    fn only_main_and_watchs_keyboard_quit_call_process_exit_directly() {
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offending = Vec::new();
        for entry in walk(&src_dir) {
            if entry.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let content = std::fs::read_to_string(&entry).expect("read source file");
            for (i, line) in content.lines().enumerate() {
                if line.contains("std::process::exit(") || line.contains("process::exit(") {
                    let rel = entry.strip_prefix(&src_dir).unwrap_or(&entry);
                    let allowed = matches!(rel.to_str(), Some("main.rs") | Some("commands/watch.rs"));
                    if !allowed {
                        offending.push(format!("{}:{}", rel.display(), i + 1));
                    }
                }
            }
        }
        assert!(
            offending.is_empty(),
            "found std::process::exit outside the allow-listed call sites (main.rs, commands/watch.rs's \
             documented keyboard-quit exception): {offending:?}"
        );
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else { return files };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walk(&path));
            } else {
                files.push(path);
            }
        }
        files
    }
}
