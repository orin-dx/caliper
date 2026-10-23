use std::io::Write;

use miette::{IntoDiagnostic, Result, WrapErr};

use crate::config::{build_options, BuildOptionsArgs};
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
        html_attributes,
        config_path,
        extra_builtins: &args.extra_builtins,
    })?;

    let pb = if !quiet && !args.json {
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner().template("{spinner:.cyan} {msg}").into_diagnostic()?);
        pb.set_message("Extracting...");
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(pb)
    } else {
        None
    };

    let output = caliper_docgen_core::pipeline::extract(&options);

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

        let json = match args.format {
            crate::OutputFormat::Canonical => serde_json::to_string_pretty(&output).into_diagnostic()?,
            crate::OutputFormat::Rdt => serialize_rdt(&output).into_diagnostic()?,
            crate::OutputFormat::Storybook => serialize_storybook(&output).into_diagnostic()?,
            crate::OutputFormat::Toon => caliper_docgen_core::toon::render_output_toon(&output),
        };

        match args.out {
            Some(ref path) => write_atomic(path, &json).into_diagnostic().wrap_err(format!("Writing to {path}"))?,
            None => println!("{json}"),
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

/// `--format rdt`: react-docgen-typescript's shape, shared with the NAPI binding.
pub fn serialize_rdt(output: &caliper_docgen_core::types::ExtractionOutput) -> serde_json::Result<String> {
    caliper_docgen_core::pipeline::rdt::serialize(output)
}

pub fn serialize_storybook(output: &caliper_docgen_core::types::ExtractionOutput) -> serde_json::Result<String> {
    let mut map = serde_json::Map::new();
    for (name, entry) in &output.components {
        let props: serde_json::Map<String, serde_json::Value> = entry
            .props
            .iter()
            .map(|(k, prop)| {
                let obj = serde_json::json!({
                    "name": prop.name,
                    "type": { "name": prop.prop_type.raw_string() },
                    "required": prop.required,
                    "defaultValue": prop.default_value.as_ref().map(|d| serde_json::json!({"value": d.value})),
                    "description": prop.description,
                });
                (k.clone(), obj)
            })
            .collect();
        map.insert(
            name.clone(),
            serde_json::json!({
                "displayName": entry.display_name,
                "props": serde_json::Value::Object(props),
                "description": entry.description,
                "methods": [],
            }),
        );
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(map))
}

#[cfg(test)]
mod tests;
