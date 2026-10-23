# Agent: NAPI (Phase 4a)

# Model: claude-sonnet-4-6

# Runs: After Phase 3 complete, parallel with Phase 4b (CLI)

# Owns: crates/napi/src/lib.rs, packages/napi/index.d.ts

## Mission

Expose three functions to Node.js: extract(), extractFile(), closeSession(). Keep the boundary thin — JSON string across the NAPI boundary, not complex NAPI types.

## The Three Functions

```rust
// crates/napi/src/lib.rs

use napi_derive::napi;
use napi::Result as NapiResult;
use std::sync::{Arc, LazyLock, atomic::{AtomicU32, Ordering}};
use dashmap::DashMap;
use camino::Utf8Path;

use oxc_react_docgen_core::pipeline::{
    extract, PipelineOptions, WatchSession, IncrementalUpdate,
};

/// Global session store — keyed by session_id.
/// Sessions persist across NAPI calls for incremental extraction.
static SESSIONS: LazyLock<DashMap<u32, Arc<WatchSession>>> = LazyLock::new(DashMap::new);
static NEXT_SESSION_ID: AtomicU32 = AtomicU32::new(1);

/// Options passed from TypeScript — flat struct for easy NAPI marshalling.
#[napi(object)]
pub struct JsExtractOptions {
    pub src_dirs: Vec<String>,
    #[napi(ts_type = "string[]")]
    pub exclude: Option<Vec<String>>,
    #[napi(ts_type = "'react18' | 'react19'")]
    pub react_version: Option<String>,
    pub cross_package: Option<bool>,
    pub pandacss_outdir: Option<String>,
    pub variant_functions: Option<Vec<String>>,
    pub skip_html_props: Option<bool>,
    // NEW fields:
    pub tsconfig_path: Option<String>,
    /// Record<string, string[]> — additional path alias mappings to merge with tsconfig paths.
    pub extra_paths: Option<serde_json::Value>,
    /// Record<string, KnownTypeOverride> — override resolution for specific type names.
    pub known_type_overrides: Option<serde_json::Value>,
    /// Extra type names to treat as built-in opaque types (in addition to the default set).
    pub extra_builtins: Option<Vec<String>>,
    /// Enable Vanilla Extract CSS-in-JS detection.
    pub vanilla_extract: Option<bool>,
    /// Directory for the DTS resolution cache.
    /// Defaults to `{project_root}/node_modules/.cache/oxc-react-docgen` — CI-friendly,
    /// matches Node.js convention. NOT the user home directory.
    pub cache_dir: Option<String>,
    /// Opt-in to resolving complex conditional/mapped types via typescript-go (future, post-v1).
    pub resolve_complex_types: Option<bool>,
}

impl From<JsExtractOptions> for PipelineOptions {
    fn from(js: JsExtractOptions) -> Self {
        PipelineOptions {
            src_dirs: js.src_dirs.into_iter()
                .map(|s| s.into())
                .collect(),
            exclude_patterns: js.exclude.unwrap_or_default(),
            react_version: match js.react_version.as_deref() {
                Some("react18") => oxc_react_docgen_core::react_types::REACT_18,
                _ => oxc_react_docgen_core::react_types::REACT_19,
            },
            cross_package: js.cross_package.unwrap_or(true),
            pandacss_outdir: js.pandacss_outdir.map(Into::into),
            variant_functions: js.variant_functions.unwrap_or_else(|| {
                vec!["cva".into(), "tv".into(), "defineRecipe".into()]
            }),
            skip_html_props: js.skip_html_props.unwrap_or(false),
            ..Default::default()
        }
    }
}

// NOTE — Cache default:
// `cache_dir` defaults to `{project_root}/node_modules/.cache/oxc-react-docgen`.
// This is CI-friendly (invalidated with node_modules) and matches Node.js convention.
// Do NOT use `dirs::cache_dir()` (user home) — it survives across CI runs and can
// cause stale cache hits when the project is rebuilt from scratch.

// NOTE — Session IDs:
// Use `AtomicU64` with `(process_id << 32) | counter` to guarantee uniqueness
// across multiple concurrent Vite dev server instances on the same machine.
// Example:
//   let pid_part = (std::process::id() as u64) << 32;
//   let counter  = NEXT_SESSION_COUNTER.fetch_add(1, Ordering::SeqCst) as u64;
//   let session_id = pid_part | counter;

/// Cold extraction — no session state. Returns JSON string.
///
/// Use this for: build-time extraction, CLI backing, one-off runs.
#[napi]
pub fn extract_all(options: JsExtractOptions) -> NapiResult<String> {
    let pipeline_options = PipelineOptions::from(options);
    let output = extract(&pipeline_options);
    serde_json::to_string(&output)
        .map_err(|e| napi::Error::from_reason(e.to_string()))
}

/// Incremental extraction — creates or reuses a session.
/// Returns JSON string of only the updated/affected components.
///
/// Use this for: Vite HMR handleHotUpdate hook.
#[napi]
pub fn extract_file_incremental(
    file_path: String,
    session_id: u32,
    options: JsExtractOptions,
) -> NapiResult<String> {
    let session = SESSIONS
        .entry(session_id)
        .or_insert_with(|| {
            Arc::new(WatchSession::new(PipelineOptions::from(options)))
        })
        .clone();

    let path = Utf8Path::new(&file_path);
    let update = session.update_file(path);

    serde_json::to_string(&update)
        .map_err(|e| napi::Error::from_reason(e.to_string()))
}

/// Create a new watch session and return its ID.
/// Call this in Vite's buildStart hook.
#[napi]
pub fn create_session(options: JsExtractOptions) -> u32 {
    let id = NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst);
    let session = Arc::new(WatchSession::new(PipelineOptions::from(options)));
    SESSIONS.insert(id, session);
    id
}

/// Release session state. Call in Vite's buildEnd hook.
#[napi]
pub fn close_session(session_id: u32) {
    SESSIONS.remove(&session_id);
}
```

## TypeScript Types (packages/napi/index.d.ts)

```typescript
export interface ExtractOptions {
  srcDirs: string[]
  exclude?: string[]
  reactVersion?: 'react18' | 'react19'
  crossPackage?: boolean
  pandacssOutdir?: string
  variantFunctions?: string[]
  skipHtmlProps?: boolean
}

export interface PropParent {
  name: string
  fileName: string
}

export interface PropItem {
  name: string
  type: PropItemType
  required: boolean
  defaultValue: { value: string; computed: boolean } | null
  description: string
  tags: Record<string, string>
  parent: PropParent | null
  declarations: PropParent[]
}

export interface PropItemType {
  kind: string
  raw?: string
  // kind-specific fields
}

export interface ComponentEntry {
  displayName: string
  filePath: string
  description: string
  props: Record<string, PropItem>
  htmlElement: string | null
  omittedHtmlProps: string[]
  composes: string[]
  tags: Record<string, string>
  methods: []
}

export interface ExtractionOutput {
  components: Record<string, ComponentEntry>
  enums: Record<string, EnumEntry[]>
  diagnostics: Diagnostic[]
  stats: ExtractionStats
}

export interface ExtractionStats {
  componentsExtracted: number
  componentSkipped: number
  filesParsed: number
  dtsCacheHits: number
  durationMs: number
}

export interface Diagnostic {
  severity: 'error' | 'warning' | 'info'
  message: string
  file?: string
  line?: number
  column?: number
  help?: string
  code: string
}

export interface IncrementalUpdate {
  updatedComponents: ComponentEntry[]
  affectedFiles: string[]
  diagnostics: Diagnostic[]
  durationMs: number
}

/** Cold extraction — no session state */
export declare function extractAll(options: ExtractOptions): string

/** Incremental extraction for HMR */
export declare function extractFileIncremental(filePath: string, sessionId: number, options: ExtractOptions): string

/** Create a watch session, returns session ID */
export declare function createSession(options: ExtractOptions): number

/** Release a watch session */
export declare function closeSession(sessionId: number): void
```

---

# Agent: CLI (Phase 4b)

# Model: claude-sonnet-4-6

# Runs: After Phase 3 complete, parallel with Phase 4a

# Owns: crates/cli/src/main.rs + sibling files

## Mission

Simple, focused CLI. Four subcommands. Rich miette diagnostics. Clean summary output. No TUI.

## CLI Structure

```rust
// crates/cli/src/main.rs

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use miette::{IntoDiagnostic, Result, WrapErr};

#[derive(Parser)]
#[command(
    name = "oxc-react-docgen",
    about = "Fast React prop extraction powered by OXC",
    version,
    propagate_version = true,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Machine-readable JSON output (suppresses human-readable output)
    #[arg(global = true, long, short = 'j')]
    json: bool,

    /// Verbose output (repeat for more: -v, -vv)
    #[arg(global = true, long, short, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress all non-error output
    #[arg(global = true, long, short)]
    quiet: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Extract prop types and write to stdout or --out file
    Extract(ExtractArgs),

    /// Watch for changes and re-extract (run in terminal alongside `storybook dev`)
    Watch(WatchArgs),

    /// Validate extraction — exits 2 if any errors. For CI.
    Check(CheckArgs),

    /// Show resolved props for a single component (debugging tool)
    Inspect(InspectArgs),

    /// Generate shell completions
    Completions(CompletionsArgs),
}

#[derive(clap::Args)]
struct ExtractArgs {
    /// Source directories to scan [default: ./src]
    #[arg(long, short, value_delimiter = ',')]
    src: Vec<String>,

    /// Output file [default: stdout]
    #[arg(long, short)]
    out: Option<String>,

    /// Output format
    #[arg(long, short, default_value = "canonical")]
    #[arg(value_enum)]
    format: OutputFormat,

    /// Disable cross-package type resolution
    #[arg(long)]
    no_cross_package: bool,

    /// React version override [default: auto-detect]
    #[arg(long)]
    react_version: Option<String>,
}

#[derive(clap::ValueEnum, Clone)]
enum OutputFormat {
    /// oxc-react-docgen canonical schema (richest)
    Canonical,
    /// react-docgen-typescript compatible output
    Rdt,
    /// Storybook __docgenInfo blocks
    Storybook,
}

#[derive(clap::Args)]
struct InspectArgs {
    /// Component name to inspect
    component: String,

    /// Source directories to scan [default: ./src]
    #[arg(long, short, value_delimiter = ',')]
    src: Vec<String>,
}

#[derive(clap::Args)]
struct WatchArgs {
    /// Source directories to watch [default: ./src]
    #[arg(long, short, value_delimiter = ',')]
    src: Vec<String>,

    /// Output file to write on each change
    #[arg(long, short)]
    out: Option<String>,
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Source directories to scan [default: ./src]
    #[arg(long, short, value_delimiter = ',')]
    src: Vec<String>,

    /// Fail on warnings in addition to errors
    #[arg(long)]
    strict: bool,
}

#[derive(clap::Args)]
struct CompletionsArgs {
    shell: Shell,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up tracing based on verbosity
    init_tracing(cli.verbose);

    match cli.command {
        Command::Extract(args) => cmd_extract(args, cli.json, cli.quiet),
        Command::Watch(args) => cmd_watch(args, cli.quiet),
        Command::Check(args) => cmd_check(args, cli.quiet),
        Command::Inspect(args) => cmd_inspect(args),
        Command::Completions(args) => cmd_completions(args),
    }
}

fn cmd_extract(args: ExtractArgs, json_mode: bool, quiet: bool) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};

    let options = build_options(&args.src, args.no_cross_package, args.react_version);

    let pb = if !quiet && !json_mode {
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap());
        pb.set_message("Extracting...");
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(pb)
    } else {
        None
    };

    let output = oxc_react_docgen_core::pipeline::extract(&options);

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    if json_mode {
        println!("{}", serde_json::to_string(&output).into_diagnostic()?);
        return Ok(());
    }

    if !quiet {
        print_summary(&output);
        print_diagnostics(&output.diagnostics);
    }

    let json = match args.format {
        OutputFormat::Canonical => serde_json::to_string_pretty(&output).into_diagnostic()?,
        OutputFormat::Rdt => serialize_rdt(&output),
        OutputFormat::Storybook => serialize_storybook(&output),
    };

    match args.out {
        Some(path) => std::fs::write(&path, &json).into_diagnostic()
            .wrap_err(format!("Writing to {}", path))?,
        None => println!("{}", json),
    }

    // Exit 2 if any errors
    if output.diagnostics.iter().any(|d| matches!(d.severity, DiagnosticSeverity::Error)) {
        std::process::exit(2);
    }

    Ok(())
}

fn print_summary(output: &ExtractionOutput) {
    use owo_colors::OwoColorize;

    println!();
    println!("  {}  oxc-react-docgen v{}", "⚡".yellow(), env!("CARGO_PKG_VERSION"));
    println!();

    let errors = output.diagnostics.iter()
        .filter(|d| matches!(d.severity, DiagnosticSeverity::Error))
        .count();
    let warnings = output.diagnostics.iter()
        .filter(|d| matches!(d.severity, DiagnosticSeverity::Warning))
        .count();

    println!("  {}  {} extracted  •  {} enums  •  {} warnings  •  {} errors",
        "Components".dimmed(),
        output.stats.components_extracted.to_string().bold(),
        output.enums.len().to_string().bold(),
        if warnings > 0 { warnings.to_string().yellow().to_string() } else { warnings.to_string() },
        if errors > 0 { errors.to_string().red().to_string() } else { errors.to_string() },
    );

    println!("  {}    {}ms total",
        "Time".dimmed(),
        output.stats.duration_ms.to_string().bold(),
    );
    println!();
}

fn cmd_inspect(args: InspectArgs) -> Result<()> {
    use owo_colors::OwoColorize;

    let options = build_options(&args.src, false, None);
    let output = oxc_react_docgen_core::pipeline::extract(&options);

    let component = output.components.get(&args.component)
        .ok_or_else(|| miette::miette!(
            "Component '{}' not found. Available: {}",
            args.component,
            output.components.keys().cloned().collect::<Vec<_>>().join(", ")
        ))?;

    println!();
    println!("  {}  {}", component.display_name.bold(), component.file_path.to_string().dimmed());
    println!("  {}", "─".repeat(60).dimmed());
    println!();

    if !component.description.is_empty() {
        println!("  {}", component.description);
        println!();
    }

    println!("  {} ({})", "Props".bold(), component.props.len());
    println!();

    for (_, prop) in &component.props {
        let type_str = prop.prop_type.raw_string();
        let required_str = if prop.required { "required".red().to_string() } else { "optional".dimmed().to_string() };
        let parent_str = prop.parent.as_ref()
            .map(|p| p.name.dimmed().to_string())
            .unwrap_or_default();

        println!("  {:<20} {:<40} {}  {}",
            prop.name.bold(),
            type_str.cyan(),
            required_str,
            parent_str,
        );

        if !prop.description.is_empty() {
            println!("  {:<20} {}", "", prop.description.dimmed());
        }
    }

    if let Some(element) = &component.html_element {
        println!();
        println!("  {}  <{}>", "HTML element".dimmed(), element.bold());
    }

    println!();
    Ok(())
}
```

## Additional CLI dependencies (`crates/cli/Cargo.toml`)

```toml
comfy-table = "7.0" # prop table in inspect command
crossterm = "0.28" # raw terminal input for watch mode (q to quit, r to re-extract)
```

## `inspect` command — table output with `comfy-table`

Replace the manual column-aligned `println!` loop above with a proper table:

```rust
fn cmd_inspect(args: InspectArgs) -> Result<()> {
    use comfy_table::{Table, Cell, Color, Attribute};

    // ... (component lookup as above) ...

    let mut table = Table::new();
    table.set_header(vec!["Prop", "Type", "Req", "Default", "From"]);

    for (_, prop) in &component.props {
        table.add_row(vec![
            Cell::new(&prop.name).add_attribute(Attribute::Bold),
            Cell::new(prop.prop_type.raw_string()).fg(Color::Cyan),
            Cell::new(if prop.required { "✓" } else { "–" }),
            Cell::new(prop.default_value.as_ref().map(|d| d.value.as_str()).unwrap_or("–")),
            Cell::new(prop.parent.as_ref().map(|p| p.name.as_str()).unwrap_or("–")).fg(Color::DarkGrey),
        ]);
    }
    println!("{table}");
    Ok(())
}
```

## Watch mode — `watchexec` 8.x API

Use the async `Watchexec` API (8.x broke the 2.x closure-based API):

```rust
use watchexec::Watchexec;

async fn cmd_watch(args: WatchArgs, quiet: bool) -> Result<()> {
    let src_dir = args.src.first().cloned().unwrap_or_else(|| "./src".into());

    let wx = Watchexec::new_async(|mut action| async move {
        for event in action.events.iter() {
            for (path, _) in event.paths() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "ts" | "tsx") {
                    // trigger re-extraction
                }
            }
        }
        action
    }).await?;
    wx.config.pathset([src_dir]);
    wx.main().await??;
    Ok(())
}
```

## Config file loading (`docgen.config.ts`)

```rust
/// Try to load docgen.config.ts from project root, walking up to workspace root.
/// Spawns node with tsx to evaluate the TS config file.
/// Returns None if file not found or node not available.
fn load_config_file(root: &Utf8Path) -> Option<PipelineOptions> {
    let config_path = find_config_file(root)?; // walks up to workspace root

    let script = format!(
        "import cfg from '{}'; process.stdout.write(JSON.stringify(cfg.default ?? cfg))",
        config_path
    );

    let output = std::process::Command::new("node")
        .args(["--input-type=module", "--import=tsx/esm"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .current_dir(root)
        .spawn()
        .ok()?
        .wait_with_output()
        .ok()?;

    if !output.status.success() { return None; }
    serde_json::from_slice(&output.stdout).ok()
}
```

The `find_config_file` helper walks from `root` upward, stopping at a workspace root marker (`pnpm-workspace.yaml`, `package.json` with `"workspaces"`, or a `.git` directory). This allows monorepo projects to place a single `docgen.config.ts` at the workspace root and have it discovered from any package.

---

# Agent: Vite Plugin (Phase 5a)

# Model: claude-sonnet-4-6

# Runs: After Phase 4a (NAPI) complete

# Owns: packages/vite-plugin/

> **⚠️ SUPERSEDED** — The architecture below (Plugin[], transform hook, __docgenInfo injection, lazy NAPI load) was revised during implementation. The actual implementation uses a single `Plugin`, virtual module pattern, and `configureServer`+`hotUpdate` hooks. See `docs/10-PLUGIN-SPEC.md` for the canonical spec and `packages/vite-plugin/src/index.ts` for the implementation. The package name is `@oxc-react-docgen/vite-plugin` (not `@oxc-react-docgen/vite`).

## Mission (original)

Thin TypeScript wrapper over the NAPI binary. Follows Vite plugin best practices. Zero re-implementation of extraction logic.

## packages/vite-plugin/src/index.ts

```typescript
import { createFilter, type FilterPattern, type Plugin } from 'vite'
import type { ExtractOptions } from '@oxc-react-docgen/napi'

// Lazy load NAPI — only available at build time (not in browser bundles)
let napi: typeof import('@oxc-react-docgen/napi') | null = null

async function getNapi() {
  if (!napi) {
    napi = await import('@oxc-react-docgen/napi')
  }
  return napi
}

export interface OxcReactDocgenOptions {
  /**
   * Glob patterns, RegExp, or string[] of files to include.
   * @default /\.(tsx|ts)$/
   */
  include?: FilterPattern

  /**
   * Glob patterns, RegExp, or string[] of files to exclude.
   * @default Automatically excludes: *.stories.*, *.test.*, *.spec.*, node_modules
   */
  exclude?: FilterPattern

  /**
   * React version for correct prop handling.
   * @default 'react19'
   */
  reactVersion?: 'react18' | 'react19'

  /**
   * Resolve types from other packages in the monorepo or node_modules.
   * @default true when workspace detected, false otherwise
   */
  crossPackage?: boolean

  /**
   * PandaCSS generated output directory.
   * @default 'styled-system' (auto-detected if panda.config.ts found)
   */
  pandacssOutdir?: string | false

  /**
   * Additional function names to treat as variant systems (like cva).
   * @default ['cva', 'tv', 'defineRecipe']
   */
  variantFunctions?: string[]

  /**
   * Filter function for props — mirrors react-docgen-typescript API.
   * Run on the TypeScript side after extraction.
   * @example propFilter: (prop) => !prop.parent?.fileName.includes('node_modules')
   */
  propFilter?: (prop: PropItem) => boolean

  /**
   * Skip inheriting HTML attributes (onClick, className, etc.) from base element.
   * @default false
   */
  skipHtmlProps?: boolean
}

// ── Vite 8 / Rolldown-compatible implementation ─────────────────────────────
//
// Key changes from a naive Vite 5/6 port:
//   - Returns Plugin[] (not Plugin) — allows splitting extraction from virtual module
//   - Uses `hotUpdate` (not `handleHotUpdate`) — Vite 8 renamed it
//   - Returns `moduleType: 'js'` in transform — required for Rolldown
//   - Uses `filter: { id: /pattern/ }` on transform hook — Rolldown perf hint
//   - Guards `this.environment?.name === 'ssr'` — skips SSR environment
//   - Extraction is fully async and non-blocking — no `extractionReady` promise gate
//   - `autoDetect()` + `loadConfigFile()` for zero-config support

export function oxcReactDocgen(inlineOptions: Options = {}): Plugin[] {
  const filter = createFilter(
    inlineOptions.include ?? /\.(tsx?|d\.ts)$/,
    inlineOptions.exclude ?? [/node_modules/, /\.stories\./, /\.test\./, /\.spec\./],
  )

  let sessionId: number | null = null
  let extractionResult: Record<string, ComponentEntry> = {}
  let extractionComplete = false
  let napiOptions: ResolvedOptions

  const extractionPlugin: Plugin = {
    name: 'oxc-react-docgen:extract',
    enforce: 'pre',

    async configResolved(config) {
      const detected = await autoDetect(config.root)
      const configFile = await loadConfigFile(config.root)
      napiOptions = mergeOptions(DEFAULTS, detected, configFile, inlineOptions).rust

      const api = await getNapi()
      sessionId = api.createSession(napiOptions)

      // Start extraction async — don't block Vite startup
      api
        .extractAll(napiOptions)
        .then((json) => {
          const output: ExtractionOutput = JSON.parse(json)
          for (const [, entry] of Object.entries(output.components)) {
            extractionResult[entry.filePath] = entry
          }
          extractionComplete = true
          // Modules will pick up __docgenInfo on next HMR cycle or page reload
        })
        .catch((err) => config.logger.warn(`[oxc-react-docgen] extraction failed: ${err}`))
    },

    // Vite 8 / Rolldown: declare filter for Rolldown perf (skips the hook for non-matching ids)
    transform: {
      filter: { id: /\.(tsx?|d\.ts)$/ },
      async handler(code, id) {
        // Skip SSR environment — __docgenInfo is client-only
        if (this.environment?.name === 'ssr') return null
        if (!filter(id)) return null

        const component = extractionResult[id]
        if (!component) {
          // Extraction may still be in progress; return unchanged
          return null
        }

        return {
          code: `${code}\n${buildDocgenBlock(component)}`,
          map: null,
          moduleType: 'js', // REQUIRED for Vite 8 / Rolldown
        }
      },
    },

    // Vite 8: hotUpdate (renamed from handleHotUpdate)
    hotUpdate({ file, environment }) {
      if (!filter(file) || sessionId === null) return
      if (environment.name !== 'client') return // skip SSR environment

      getNapi().then((api) => {
        const json = api.extractFileIncremental(file, sessionId!, napiOptions)
        const update: IncrementalUpdate = JSON.parse(json)
        for (const entry of update.updatedComponents) {
          extractionResult[entry.filePath] = entry
        }
        // Return invalidated modules to Vite for HMR
        return update.affectedFiles
          .map((f) => environment.moduleGraph.getModuleById(f))
          .filter((m): m is NonNullable<typeof m> => m != null)
      })
    },

    buildEnd() {
      if (sessionId !== null) {
        getNapi().then((api) => api.closeSession(sessionId!))
        sessionId = null
      }
    },
  }

  return [extractionPlugin, virtualModulePlugin(/* see virtual module plugin spec below */)]
}

function buildDocgenBlock(component: ComponentEntry): string {
  // HTML-safe JSON escaping for __docgenInfo
  const json = JSON.stringify(component).replace(/</g, '\\u003c').replace(/>/g, '\\u003e').replace(/&/g, '\\u0026')

  return [
    `if (typeof ${component.displayName} !== 'undefined') {`,
    `  ${component.displayName}.__docgenInfo = ${json}`,
    `}`,
  ].join('\n')
}

// Re-export types for downstream TypeScript users
export type { ComponentEntry, PropItem, ExtractionOutput } from '@oxc-react-docgen/napi'
```

## packages/vite-plugin/package.json

```json
{
  "name": "@oxc-react-docgen/vite",
  "version": "0.1.0",
  "description": "Vite plugin for oxc-react-docgen",
  "main": "./dist/index.js",
  "module": "./dist/index.mjs",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": {
      "import": "./dist/index.mjs",
      "require": "./dist/index.js",
      "types": "./dist/index.d.ts"
    }
  },
  "peerDependencies": {
    "vite": ">=5.0.0 || ^8.0.0"
  },
  "dependencies": {
    "@oxc-react-docgen/napi": "workspace:*"
  },
  "devDependencies": {
    "vite": "^6.0.0",
    "typescript": "^5.5.0",
    "tsup": "^8.0.0"
  },
  "scripts": {
    "build": "tsup src/index.ts --format cjs,esm --dts",
    "dev": "tsup src/index.ts --format cjs,esm --dts --watch"
  }
}
```

---

# Agent: Rolldown Plugin (Phase 5b)

# Model: claude-sonnet-4-6

# Runs: After Phase 3 complete (no NAPI needed — uses core directly)

# Owns: packages/rolldown-plugin/ (Rust crate)

## Mission

Native Rust rolldown plugin — no NAPI, no JSON boundary. Uses the core crate directly. Follows rolldown plugin API.

## packages/rolldown-plugin/Cargo.toml

```toml
[package]
name = "oxc-react-docgen-rolldown"
version.workspace = true
edition.workspace = true

[lib]
crate-type = ["cdylib"]

[dependencies]
oxc-react-docgen-core = { path = "../../crates/core" }
# rolldown plugin API — import when stabilized
# rolldown_plugin = "0.1"
```

## packages/rolldown-plugin/src/lib.rs

```rust
use std::sync::Arc;
use oxc_react_docgen_core::pipeline::{extract, PipelineOptions, WatchSession};
use oxc_react_docgen_core::types::ComponentEntry;

/// Native Rolldown plugin for oxc-react-docgen.
/// Uses the core crate directly — no NAPI boundary.
pub struct OxcReactDocgenPlugin {
    options: PipelineOptions,
    // Component cache — populated in build_start, read in transform
    components: Arc<dashmap::DashMap<String, ComponentEntry>>,
}

impl OxcReactDocgenPlugin {
    pub fn new(options: PipelineOptions) -> Self {
        Self {
            options,
            components: Arc::new(dashmap::DashMap::new()),
        }
    }
}

// NOTE: Rolldown native plugin API is still stabilizing.
// Implement `rolldown_plugin::Plugin` trait when the API is stable.
// For now, expose the builder and let the TS rolldown plugin call NAPI
// (same as Vite plugin) until the native API stabilizes.

/// Builder for ergonomic configuration from Rust code.
pub struct OxcReactDocgenPluginBuilder {
    options: PipelineOptions,
}

impl OxcReactDocgenPluginBuilder {
    pub fn new() -> Self {
        Self { options: PipelineOptions::default() }
    }

    pub fn src_dirs(mut self, dirs: Vec<impl Into<camino::Utf8PathBuf>>) -> Self {
        self.options.src_dirs = dirs.into_iter().map(Into::into).collect();
        self
    }

    pub fn react_version(mut self, v: oxc_react_docgen_core::react_types::ReactVersion) -> Self {
        self.options.react_version = v;
        self
    }

    pub fn build(self) -> OxcReactDocgenPlugin {
        OxcReactDocgenPlugin::new(self.options)
    }
}
```
