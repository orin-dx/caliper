//! The `caliper-docgen` command line: `extract`, `watch`, `check`, `inspect`, `schema` and `completions` over the
//! `caliper-docgen-core` pipeline. The command handlers return their exit code and `main` is the only place that ends
//! the process.

#![deny(missing_docs)]
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

/// The command line: the flags every command shares, and the command to run.
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

/// The options of `extract`.
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

    /// Which inherited sources to list as one inherited layer of names instead of prop by prop: large ambient ones
    /// (DOM and ARIA attributes, style systems) from loaded dependencies by default; a dependency's own API is listed unless it declares more than 200 props
    #[arg(long, value_enum)]
    pub collapse_inherited_props: Option<CollapseInheritedPropsArg>,

    /// Experimental: ask the built-in TypeScript 7.1 checker (`tsrs`) for every component's props in one program and
    /// wait for the answer, which replaces the structural one prop by prop; the structural result stands wherever the
    /// checker cannot answer (off by default)
    #[arg(long)]
    pub experimental_semantic_types: bool,

    /// A checker binary for `--experimental-semantic-types` to run instead of the built-in one: `typescript@7.1`'s
    /// `tsc`, or `tsrs`
    #[arg(long, requires = "experimental_semantic_types")]
    pub tsc_path: Option<String>,

    /// With `--experimental-semantic-types`: how long the checker may take to start or to answer any one request, in
    /// milliseconds, before the run takes the structural result with one warning
    #[arg(long, requires = "experimental_semantic_types", default_value_t = DEFAULT_SEMANTIC_TIMEOUT_MS)]
    pub semantic_timeout_ms: u64,

    /// Test-only (ADR 0026's S7 gates): with `--experimental-semantic-types`, take no fact the checker lacks from the
    /// structural resolver: defaults and union member order come from the source's syntax alone
    #[arg(long, hide = true, requires = "experimental_semantic_types")]
    pub no_resolver_facts: bool,
}

/// `--semantic-timeout-ms` when not given: ADR 0026's proposed 10 s. A hung checker is caught that fast, and it bounds
/// each request, not the run: a 938-component Chakra and MUI program answers in 15 to 17 s in all within it.
pub const DEFAULT_SEMANTIC_TIMEOUT_MS: u64 = 10_000;

/// The formats `extract` can write.
#[derive(clap::ValueEnum, Clone)]
pub enum OutputFormat {
    /// caliper-docgen canonical schema (richest)
    Canonical,
    /// react-docgen-typescript compatible output
    Rdt,
    /// Token-Optimized Object Notation (compact agent format)
    Toon,
}

/// The values of `--html-attributes`: how much of an inherited HTML element's attribute set a component lists.
#[derive(clap::ValueEnum, Clone, Copy)]
pub enum HtmlAttributeModeArg {
    /// A compact subset per element: handlers, aria-*, ref and the common attributes
    Curated,
    /// All of `@types/react`'s real attributes for the element (matches RDT) (default)
    Full,
    /// No inherited HTML attributes at all — own props only
    None,
}

/// The values of `--collapse-inherited-props`: which inherited sources are listed as one inherited layer of names
/// instead of prop by prop.
#[derive(clap::ValueEnum, Clone, Copy)]
pub enum CollapseInheritedPropsArg {
    /// Large ambient sources (DOM and ARIA attributes, style systems) from loaded dependencies (default)
    Dependencies,
    /// Those, and project sources of more than 30 props
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

/// The options of `inspect`.
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

    /// Which inherited sources to list as one inherited layer of names instead of prop by prop: large ambient ones
    /// (DOM and ARIA attributes, style systems) from loaded dependencies by default; a dependency's own API is listed unless it declares more than 200 props
    #[arg(long, value_enum)]
    pub collapse_inherited_props: Option<CollapseInheritedPropsArg>,
}

/// The options of `watch`.
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

    /// Which inherited sources to list as one inherited layer of names instead of prop by prop: large ambient ones
    /// (DOM and ARIA attributes, style systems) from loaded dependencies by default; a dependency's own API is listed unless it declares more than 200 props
    #[arg(long, value_enum)]
    pub collapse_inherited_props: Option<CollapseInheritedPropsArg>,
}

/// The options of `check`.
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

    /// Which inherited sources to list as one inherited layer of names instead of prop by prop: large ambient ones
    /// (DOM and ARIA attributes, style systems) from loaded dependencies by default; a dependency's own API is listed unless it declares more than 200 props
    #[arg(long, value_enum)]
    pub collapse_inherited_props: Option<CollapseInheritedPropsArg>,
}

/// The options of `completions`.
#[derive(clap::Args)]
pub struct CompletionsArgs {
    /// The shell to generate completions for
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
    use tracing_subscriber::{EnvFilter, fmt};
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
}
