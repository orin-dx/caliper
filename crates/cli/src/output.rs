/// Writes `contents` to `path` via a same-directory temp file + rename, so a
/// mid-write failure (disk full, permission revoked) can never leave `path`
/// truncated or half-written. Returns the `io::Error` on failure instead of
/// swallowing it — callers must report it, not discard the `Result`.
///
/// A target that exists but is not a regular file (`/dev/null`, a FIFO, `/dev/stdout`) is written in place: renaming
/// over it would replace the device with a file, and its directory is often not writable.
pub fn write_atomic(path: &str, contents: &str) -> std::io::Result<()> {
    let target = std::path::Path::new(path);
    if std::fs::metadata(target).is_ok_and(|m| !m.is_file() && !m.is_dir()) {
        return std::fs::write(target, contents);
    }
    let dir = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    let file_name = target.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("'{path}' has no file name component"))
    })?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(".tmp");
    let tmp_path = dir.join(tmp_name);
    if let Err(e) = std::fs::write(&tmp_path, contents) {
        std::fs::remove_file(&tmp_path).ok();
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp_path, target) {
        std::fs::remove_file(&tmp_path).ok();
        return Err(e);
    }
    Ok(())
}

/// Human-readable extraction summary. Always written to stderr — stdout is reserved for the
/// JSON payload (canonical/RDT/storybook), in every mode, so `caliper-docgen extract | jq .`
/// never sees this interleaved with the data.
pub fn print_summary(output: &caliper_docgen_core::types::ExtractionOutput, quiet: bool) {
    if quiet {
        return;
    }
    // A closed stderr isn't worth failing a run over.
    write_summary(&mut std::io::stderr().lock(), output).ok();
}

fn write_summary(
    w: &mut impl std::io::Write,
    output: &caliper_docgen_core::types::ExtractionOutput,
) -> std::io::Result<()> {
    use owo_colors::OwoColorize;

    let errors = output
        .diagnostics
        .iter()
        .filter(|d| matches!(d.severity, caliper_docgen_core::types::DiagnosticSeverity::Error))
        .count();
    let warnings = output
        .diagnostics
        .iter()
        .filter(|d| matches!(d.severity, caliper_docgen_core::types::DiagnosticSeverity::Warning))
        .count();

    writeln!(w)?;
    writeln!(
        w,
        "  {}  {} components  ·  {} enums  ·  {}  ·  {}  ·  {}ms",
        "⚡".yellow(),
        output.stats.components_extracted.to_string().bold(),
        output.enums.len().to_string().bold(),
        if warnings > 0 { format!("{warnings} warnings").yellow().to_string() } else { format!("{warnings} warnings") },
        if errors > 0 { format!("{errors} errors").red().to_string() } else { format!("{errors} errors") },
        output.stats.duration_ms.to_string().bold(),
    )?;
    writeln!(w)
}

/// Human-readable diagnostic list. Always written to stderr — see [`print_summary`].
pub fn print_diagnostics(diagnostics: &[caliper_docgen_core::types::Diagnostic]) {
    write_diagnostics(&mut std::io::stderr().lock(), diagnostics).ok();
}

fn write_diagnostics(
    w: &mut impl std::io::Write,
    diagnostics: &[caliper_docgen_core::types::Diagnostic],
) -> std::io::Result<()> {
    for line in format_diagnostics(diagnostics) {
        writeln!(w, "{line}")?;
    }
    Ok(())
}

/// The text between the first pair of single quotes in `message`, e.g. `Date` from
/// `"Cannot resolve type 'Date' in 'a.ts'"`. Falls back to the full message when there's
/// no quoted substring, so unrelated diagnostics never collide on an empty subject.
fn extract_subject(message: &str) -> &str {
    let Some(start) = message.find('\'') else {
        return message;
    };
    let rest = &message[start + 1..];
    match rest.find('\'') {
        Some(len) => &rest[..len],
        None => message,
    }
}

/// One or more diagnostics collapsed into a single reported group, keyed by `(code, subject)`.
struct DiagnosticGroup<'a> {
    representative: &'a caliper_docgen_core::types::Diagnostic,
    count: usize,
    files_seen: Vec<Option<&'a str>>,
}

/// Collapses near-duplicate diagnostics (same code + same quoted subject) into groups, so a
/// single root cause that produced hundreds of diagnostics reports as one line with a count
/// instead of flooding the terminal. Order is preserved as first-seen, per group.
fn group_diagnostics(diagnostics: &[caliper_docgen_core::types::Diagnostic]) -> Vec<DiagnosticGroup<'_>> {
    let mut groups: Vec<(&caliper_docgen_core::types::DiagnosticCode, &str, DiagnosticGroup)> = Vec::new();
    for d in diagnostics {
        let subject = extract_subject(&d.message);
        // A deduplicated diagnostic stands for one occurrence per component it affects.
        let occurrences = d.affects.len().max(1);
        match groups.iter_mut().find(|(code, s, _)| *code == &d.code && *s == subject) {
            Some((_, _, group)) => {
                group.count += occurrences;
                if !group.files_seen.contains(&d.file.as_deref()) {
                    group.files_seen.push(d.file.as_deref());
                }
            }
            None => groups.push((
                &d.code,
                subject,
                DiagnosticGroup { representative: d, count: occurrences, files_seen: vec![d.file.as_deref()] },
            )),
        }
    }
    let mut groups: Vec<DiagnosticGroup> = groups.into_iter().map(|(_, _, group)| group).collect();
    groups.sort_by_key(|group| std::cmp::Reverse(group.count));
    groups
}

/// Formats grouped diagnostics into the lines `print_diagnostics` writes to stderr. Pulled out
/// as a pure function so grouping/formatting behavior is unit-testable without capturing stderr.
fn format_diagnostics(diagnostics: &[caliper_docgen_core::types::Diagnostic]) -> Vec<String> {
    use owo_colors::OwoColorize;

    let mut lines = Vec::new();
    for group in group_diagnostics(diagnostics) {
        let d = group.representative;
        let prefix = match d.severity {
            caliper_docgen_core::types::DiagnosticSeverity::Error => "error".red().to_string(),
            caliper_docgen_core::types::DiagnosticSeverity::Warning => "warn".yellow().to_string(),
            caliper_docgen_core::types::DiagnosticSeverity::Info => "info".dimmed().to_string(),
            _ => "info".dimmed().to_string(),
        };

        let suffix = match (group.count, group.files_seen.len()) {
            (1, _) => String::new(),
            (count, 1) => format!("  (×{count})"),
            (count, files) => format!("  (×{count}, {files} files)"),
        };

        if let Some(ref file) = d.file {
            lines.push(format!("  [{prefix}] {file}:{}{suffix}", d.message));
        } else {
            lines.push(format!("  [{prefix}] {}{suffix}", d.message));
        }
        if let Some(ref help) = d.help {
            lines.push(format!("    {} {}", "help:".dimmed(), help));
        }
    }
    lines
}

/// The `inspect` view of one component: header, description, props table, and the inherited-attributes note.
pub fn format_component(component: &caliper_docgen_core::types::ComponentEntry) -> Vec<String> {
    use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table};
    use owo_colors::OwoColorize;

    let mut lines = vec![
        String::new(),
        format!("  {}  {}", component.display_name.bold(), component.file_path.to_string().dimmed()),
        format!("  {}", "─".repeat(70).dimmed()),
    ];
    if !component.description.is_empty() {
        lines.push(String::new());
        lines.push(format!("  {}", component.description));
    }
    lines.push(String::new());
    lines.push(format!("  {} ({})", "Props".bold(), component.props.len()));
    lines.push(String::new());

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("Prop").add_attribute(Attribute::Bold),
        Cell::new("Type").add_attribute(Attribute::Bold),
        Cell::new("Req").add_attribute(Attribute::Bold),
        Cell::new("Default").add_attribute(Attribute::Bold),
        Cell::new("From").add_attribute(Attribute::Bold),
    ]);
    for prop in component.props.values() {
        let req_str = if prop.required { "✓".to_string() } else { "–".to_string() };
        let default_str = prop.default_value.as_ref().map(|d| d.value.clone()).unwrap_or_else(|| "–".into());
        let from_str = prop.parent.as_ref().map(|p| p.name.clone()).unwrap_or_default();

        table.add_row(vec![
            Cell::new(&prop.name).fg(Color::White),
            Cell::new(prop.prop_type.raw_string()).fg(Color::Cyan),
            Cell::new(&req_str),
            Cell::new(&default_str).fg(Color::DarkGrey),
            Cell::new(&from_str).fg(Color::DarkGrey),
        ]);
    }
    lines.extend(table.to_string().lines().map(|line| format!("  {line}")));

    if !component.notable_inherited.is_empty() {
        lines.push(String::new());
        for layer in &component.inheritance {
            let element_note = layer.html_element.as_ref().map(|e| format!(" (<{e}>)")).unwrap_or_default();
            lines.push(format!("  {} {}{}", "↳".dimmed(), layer.type_name.dimmed(), element_note.dimmed()));
        }
        let notable_names: Vec<&str> = component.notable_inherited.keys().map(|s| s.as_str()).collect();
        lines.push(format!("    Notable: {}", notable_names.join("  ").dimmed()));
    }
    lines.push(String::new());
    lines
}

pub fn print_component(component: &caliper_docgen_core::types::ComponentEntry) {
    for line in format_component(component) {
        println!("{line}");
    }
}

/// Drops SGR colour sequences so tests can assert on plain text.
#[cfg(test)]
pub(crate) fn strip_ansi(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            chars.by_ref().find(|&c| c == 'm');
        } else {
            plain.push(c);
        }
    }
    plain
}

#[cfg(test)]
mod tests;
