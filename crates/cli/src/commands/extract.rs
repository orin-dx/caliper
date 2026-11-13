use std::io::Write;

use miette::{IntoDiagnostic, Result, WrapErr};

use crate::config::{BuildOptionsArgs, build_options};
use crate::output::{print_diagnostics, print_summary, write_atomic};

/// Returns the process exit code (0 = success, 2 = extraction reported an
/// error-severity diagnostic) rather than calling `std::process::exit`
/// directly — keeps the exit decision testable and lets `main()` be the only
/// place that actually terminates the process.
pub fn cmd_extract(args: crate::ExtractArgs, quiet: bool, config_path: Option<&str>) -> Result<i32> {
    use indicatif::{ProgressBar, ProgressStyle};

    let html_attributes = args.html_attributes.map(|m| match m {
        crate::HtmlAttributeModeArg::Curated => "curated",
        crate::HtmlAttributeModeArg::Full => "full",
        crate::HtmlAttributeModeArg::None => "none",
    });
    let options = build_options(BuildOptionsArgs {
        src: &args.src,
        cache_dir: args.cache_dir.as_deref(),
        no_cache: args.no_cache,
        html_attributes,
        config_path,
        extra_builtins: &args.extra_builtins,
        load_dependencies: args.load_dependencies,
        no_load_dependencies: args.no_load_dependencies,
        collapse_inherited_props: args.collapse_inherited_props.map(Into::into),
    })?;
    let mut options = options;
    if args.no_checker {
        options.checker = false;
    }

    let pb = if !quiet && !args.json {
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner().template("{spinner:.cyan} {msg}").into_diagnostic()?);
        pb.set_message("Extracting...");
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(pb)
    } else {
        None
    };

    let mut output = caliper_docgen_core::pipeline::extract(&options);
    if options.checker {
        let resolver = match args.no_resolver_facts {
            true => caliper_docgen_core::types::semantic::ResolverFacts::Off,
            false => caliper_docgen_core::types::semantic::ResolverFacts::Used,
        };
        let timeout = std::time::Duration::from_millis(args.semantic_timeout_ms);
        apply_semantic_backend(
            &options,
            timeout,
            resolver,
            None,
            caliper_docgen_semantic::Teardown::OneShot,
            &mut output,
        );
    }

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    // --json ignores --out/--format — see ExtractArgs::json's doc comment.
    if args.json {
        let mut stdout = std::io::BufWriter::with_capacity(1 << 16, std::io::stdout().lock());
        caliper_docgen_core::pipeline::write_extraction_output_json(&output, &mut stdout).into_diagnostic()?;
        writeln!(stdout).and_then(|()| stdout.flush()).into_diagnostic()?;
    } else {
        if !quiet {
            print_summary(&output, quiet);
            print_diagnostics(&output.diagnostics);
        }

        if matches!(args.format, crate::OutputFormat::Rdt) && args.out.is_none() {
            // The document is written as it is rendered, not built whole first.
            let mut stdout = std::io::BufWriter::with_capacity(1 << 16, std::io::stdout().lock());
            caliper_docgen_core::pipeline::rdt::write(&output, args.include_unexported, &mut stdout)
                .into_diagnostic()?;
            writeln!(stdout).and_then(|()| stdout.flush()).into_diagnostic()?;
        } else {
            let json = match args.format {
                crate::OutputFormat::Canonical => serde_json::to_string_pretty(&output).into_diagnostic()?,
                crate::OutputFormat::Rdt => serialize_rdt(&output, args.include_unexported).into_diagnostic()?,
                crate::OutputFormat::Toon => caliper_docgen_core::toon::render_output_toon(&output),
            };

            match args.out {
                Some(ref path) => write_atomic(path, &json).into_diagnostic().wrap_err(format!("Writing to {path}"))?,
                None => println!("{json}"),
            }
        }
    }

    // Must run regardless of --json — this is the one thing CI actually
    // depends on the exit code for.
    let exit_code = output.exit_code(false);
    // The process ends right after this, and freeing a large project's output one allocation at a time is slower
    // than letting the OS reclaim it.
    std::mem::forget(output);
    Ok(exit_code)
}

/// Resolve the components the structural pass found with TypeScript's checker, all in one program, and merge the answer
/// over `output`; with `only`, just the component under that key. The run waits for the checker; one request (or its
/// start) that takes longer than `timeout`, like an unavailable checker, leaves `output` as it was, plus one
/// diagnostic. `teardown` is whether the process ends with the run ([`caliper_docgen_semantic::Teardown`]).
fn apply_semantic_backend(
    options: &caliper_docgen_core::pipeline::PipelineOptions,
    timeout: std::time::Duration,
    resolver: caliper_docgen_core::types::semantic::ResolverFacts,
    only: Option<&str>,
    teardown: caliper_docgen_semantic::Teardown,
    output: &mut caliper_docgen_core::types::ExtractionOutput,
) {
    let project_root = options
        .src_dirs
        .first()
        .and_then(|dir| caliper_docgen_core::canonical_path::canonicalize(dir).ok())
        .or_else(|| std::env::current_dir().ok().and_then(|dir| camino::Utf8PathBuf::from_path_buf(dir).ok()));
    let Some(project_root) = project_root else {
        let outcome = caliper_docgen_core::types::semantic::SemanticOutcome::unavailable("no project directory");
        caliper_docgen_core::types::semantic::merge_semantic(output, outcome);
        return;
    };
    let mut targets = caliper_docgen_semantic::targets_of(output);
    if let Some(key) = only {
        targets.retain(|target| target.key == key);
    }
    let mut request = caliper_docgen_semantic::SemanticRequest::new(project_root, targets);
    request.tsconfig = options
        .tsconfig_path
        .clone()
        .or_else(|| request.project_root.ancestors().map(|dir| dir.join("tsconfig.json")).find(|file| file.is_file()));
    request.request_timeout = timeout;
    let request = request.with_teardown(teardown);
    let outcome = caliper_docgen_semantic::resolve(request);
    caliper_docgen_core::types::semantic::merge_semantic_with(output, outcome, resolver);
}

/// What `check` and `inspect` run: unless `--no-checker` or the config turned it off, [`apply_semantic_backend`] at the
/// default timeout, with the resolver's facts, for `only` the component under that key when given.
pub(crate) fn apply_checker(
    options: &caliper_docgen_core::pipeline::PipelineOptions,
    no_checker: bool,
    only: Option<&str>,
    output: &mut caliper_docgen_core::types::ExtractionOutput,
) {
    if options.checker && !no_checker {
        let timeout = std::time::Duration::from_millis(crate::DEFAULT_SEMANTIC_TIMEOUT_MS);
        let resolver = caliper_docgen_core::types::semantic::ResolverFacts::Used;
        apply_semantic_backend(options, timeout, resolver, only, caliper_docgen_semantic::Teardown::Session, output);
    }
}

/// `--format rdt`: react-docgen-typescript's shape, shared with the NAPI binding. Like react-docgen-typescript, only
/// exported components unless asked.
pub fn serialize_rdt(
    output: &caliper_docgen_core::types::ExtractionOutput,
    include_unexported: bool,
) -> serde_json::Result<String> {
    caliper_docgen_core::pipeline::rdt::serialize(output, include_unexported)
}

#[cfg(test)]
mod tests;
