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
        load_dependencies: args.load_dependencies,
        collapse_inherited_props: args.collapse_inherited_props.map(Into::into),
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
            crate::OutputFormat::Rdt => serialize_rdt(&output, args.include_unexported).into_diagnostic()?,
            crate::OutputFormat::Storybook => {
                serialize_storybook(&output, args.include_unexported).into_diagnostic()?
            }
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

/// RDT's type-name convention for literal unions: `{"name": "enum", "value": [...]}`
/// instead of inlining the literal text into `type.name` as a plain string. Lets
/// prop-table renderers that pattern-match `type.name === "enum"` (a common
/// Storybook addon integration point) show a `<select>` control for the most
/// common, most curated props in any design system (variant, size, color…).
fn rdt_type_json(prop_type: &caliper_docgen_core::types::PropType) -> serde_json::Value {
    use caliper_docgen_core::types::PropType;

    if prop_type.is_literal_union() {
        let values: Vec<serde_json::Value> = match prop_type {
            PropType::Union(members) => members.iter().map(|m| serde_json::json!({"value": m.raw_string()})).collect(),
            PropType::LiteralUnion { members, .. } => {
                members.iter().map(|m| serde_json::json!({"value": format!("\"{m}\"")})).collect()
            }
            _ => vec![],
        };
        return serde_json::json!({ "name": "enum", "value": values });
    }
    serde_json::json!({ "name": prop_type.raw_string() })
}

/// RDT's default `skipChildrenPropWithoutDoc`: an undocumented `children` says nothing a
/// prop table can show, so RDT never emits it. Canonical output keeps it.
fn rdt_omits(prop: &caliper_docgen_core::types::ParsedProp) -> bool {
    prop.name == "children" && prop.description.is_empty()
}

/// The components a compatibility format writes: like react-docgen-typescript, only exported ones unless asked.
fn compat_components(
    output: &caliper_docgen_core::types::ExtractionOutput,
    include_unexported: bool,
) -> impl Iterator<Item = (&String, &caliper_docgen_core::types::ComponentEntry)> {
    output.components.iter().filter(move |(_, entry)| include_unexported || entry.exported)
}

/// `methods` is always `[]` — this tool doesn't extract class methods, and RDT
/// consumers (Storybook's docgen addon) only ever read it for class components.
pub fn serialize_rdt(
    output: &caliper_docgen_core::types::ExtractionOutput,
    include_unexported: bool,
) -> serde_json::Result<String> {
    let mut map = serde_json::Map::new();
    for (name, entry) in compat_components(output, include_unexported) {
        let props: serde_json::Map<String, serde_json::Value> = entry
            .props
            .iter()
            .filter(|(_, prop)| !rdt_omits(prop))
            .map(|(k, prop)| {
                let obj = serde_json::json!({
                    "name": prop.name,
                    "type": rdt_type_json(&prop.prop_type),
                    "required": prop.required,
                    "defaultValue": prop.default_value.as_ref().map(|d| serde_json::json!({"value": d.value, "computed": d.computed})),
                    "description": prop.description,
                    "parent": prop.parent.as_ref().map(|p| serde_json::json!({"name": p.name, "fileName": p.file_name})),
                });
                (k.clone(), obj)
            })
            .collect();
        map.insert(
            name.clone(),
            serde_json::json!({
                "displayName": entry.display_name,
                "filePath": entry.file_path,
                "props": serde_json::Value::Object(props),
                "description": entry.description,
                "methods": [],
                "tags": entry.tags,
                "composes": entry.composes,
            }),
        );
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(map))
}

pub fn serialize_storybook(
    output: &caliper_docgen_core::types::ExtractionOutput,
    include_unexported: bool,
) -> serde_json::Result<String> {
    let mut map = serde_json::Map::new();
    for (name, entry) in compat_components(output, include_unexported) {
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
