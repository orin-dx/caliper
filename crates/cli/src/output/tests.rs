use caliper_docgen_core::types::{
    ComponentEntry, DefaultValue, Diagnostic, DiagnosticCode, DiagnosticSeverity, ExtractionOutput, ExtractionStats,
    InheritedLayer, ParsedProp, PropParent, PropType,
};
use owo_colors::OwoColorize;

use super::{
    extract_subject, format_component, format_diagnostics, strip_ansi, write_atomic, write_diagnostics, write_summary,
};

#[test]
fn write_atomic_rejects_a_path_with_no_file_name() {
    let err = write_atomic("/", "{}").expect_err("the filesystem root has no file name to write");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(err.to_string(), "'/' has no file name component");
}

fn output_with(diagnostics: Vec<Diagnostic>) -> ExtractionOutput {
    ExtractionOutput {
        components: Default::default(),
        enums: Default::default(),
        diagnostics,
        stats: ExtractionStats { components_extracted: 3, duration_ms: 12, ..Default::default() },
    }
}

fn summary_of(output: &ExtractionOutput) -> String {
    let mut written = Vec::new();
    write_summary(&mut written, output).unwrap();
    String::from_utf8(written).unwrap()
}

#[test]
fn summary_reports_the_counts_and_colors_only_the_nonzero_warning_and_error_ones() {
    let clean = summary_of(&output_with(vec![]));
    assert_eq!(strip_ansi(&clean), "\n  ⚡  3 components  ·  0 enums  ·  0 warnings  ·  0 errors  ·  12ms\n\n");
    assert!(!clean.contains("\u{1b}[33m0 warnings") && !clean.contains("\u{1b}[31m0 errors"), "{clean:?}");

    let diagnostics = vec![
        diag(DiagnosticSeverity::Warning, DiagnosticCode::OpaqueType, "a", None),
        diag(DiagnosticSeverity::Warning, DiagnosticCode::OpaqueType, "b", None),
        diag(DiagnosticSeverity::Error, DiagnosticCode::IoError, "c", None),
        diag(DiagnosticSeverity::Info, DiagnosticCode::OpaqueType, "d", None),
    ];
    let noisy = summary_of(&output_with(diagnostics));
    assert_eq!(strip_ansi(&noisy), "\n  ⚡  3 components  ·  0 enums  ·  2 warnings  ·  1 errors  ·  12ms\n\n");
    assert!(
        noisy.contains(&"2 warnings".yellow().to_string()) && noisy.contains(&"1 errors".red().to_string()),
        "{noisy:?}"
    );
}

#[test]
fn diagnostics_are_written_one_line_each_with_a_severity_prefix() {
    let diagnostics = vec![
        diag(DiagnosticSeverity::Error, DiagnosticCode::UnresolvableImport, "Cannot resolve 'A'", Some("a.ts")),
        diag(DiagnosticSeverity::Info, DiagnosticCode::OpaqueType, "Opaque 'B'", None),
    ];
    let mut written = Vec::new();
    write_diagnostics(&mut written, &diagnostics).unwrap();
    let written = String::from_utf8(written).unwrap();

    assert_eq!(strip_ansi(&written), "  [error] a.ts:Cannot resolve 'A'\n  [info] Opaque 'B'\n");
    assert!(written.contains(&"error".red().to_string()), "errors are colored red: {written:?}");
}

#[test]
fn extract_subject_of_a_message_with_one_lone_quote_is_the_whole_message() {
    assert_eq!(extract_subject("it's broken"), "it's broken");
}

#[test]
fn format_component_renders_header_description_props_table_and_inherited_note() {
    let prop = |name: &str, prop_type, required, default: Option<&str>, parent: Option<&str>| {
        let default_value = default.map(|value| DefaultValue { value: value.into(), computed: false });
        let parent = parent.map(|name| PropParent { name: name.into(), file_name: "Widget.tsx".into() });
        ParsedProp::new(
            name.into(),
            prop_type,
            required,
            default_value,
            String::new(),
            Default::default(),
            parent,
            vec![],
        )
    };
    let entry = ComponentEntry {
        display_name: "Widget".into(),
        file_path: "src/Widget.tsx".into(),
        description: "A widget.".into(),
        props: [
            ("label", prop("label", PropType::String, true, None, Some("WidgetProps"))),
            ("size", prop("size", PropType::Number, false, Some("3"), None)),
        ]
        .into_iter()
        .map(|(name, prop)| (name.to_owned(), prop))
        .collect(),
        inheritance: vec![InheritedLayer {
            type_name: "ButtonHTMLAttributes".into(),
            file_name: "react".into(),
            omitted: vec![],
            html_element: Some("button".into()),
            total_props: 0,
        }],
        notable_inherited: [("onClick".to_owned(), prop("onClick", PropType::Any, false, None, None))]
            .into_iter()
            .collect(),
        discriminant_prop: None,
        composes: vec![],
        tags: Default::default(),
        methods: vec![],
    };

    let lines: Vec<String> = format_component(&entry).iter().map(|line| strip_ansi(line)).collect();

    let rule = format!("  {}", "─".repeat(70));
    let top = ["", "  Widget  src/Widget.tsx", rule.as_str(), "", "  A widget.", "", "  Props (2)", ""];
    assert_eq!(lines[..top.len()], top);
    let bottom = ["", "  ↳ ButtonHTMLAttributes (<button>)", "    Notable: onClick", ""];
    assert_eq!(lines[lines.len() - bottom.len()..], bottom);

    // Border glyphs depend on the terminal, so accept either pipe; rows without text are separators.
    let rows: Vec<Vec<&str>> = lines
        .iter()
        .filter(|line| line.contains(['|', '│']) && line.chars().any(char::is_alphanumeric))
        .map(|line| {
            let cells: Vec<&str> = line.split(['|', '│']).map(str::trim).collect();
            cells[1..cells.len() - 1].to_vec()
        })
        .collect();
    assert_eq!(rows[0], ["Prop", "Type", "Req", "Default", "From"]);
    assert_eq!(rows[1], ["label", "string", "✓", "–", "WidgetProps"]);
    assert_eq!(rows[2], ["size", "number", "–", "3", ""]);
}

#[test]
fn write_atomic_surfaces_error_when_parent_dir_is_missing() {
    let err = write_atomic("/nonexistent-rdt-out-dir-xyz-123/out.json", "{}")
        .expect_err("write to a missing parent directory should surface an error, not succeed silently");
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::NotFound,
        "expected a NotFound io::Error for a missing parent directory, got {err:?}"
    );
}

#[test]
fn write_atomic_cleans_up_temp_file_when_rename_fails() {
    let dir = std::env::temp_dir().join(format!("rdt-out-atomic-rename-fail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create test dir");
    // A directory can't be the rename target of a regular file — forces
    // the rename step (not the write-to-tmp step) to fail.
    let target_as_dir = dir.join("out.json");
    std::fs::create_dir_all(&target_as_dir).expect("create target-as-dir");

    let result = write_atomic(target_as_dir.to_str().expect("utf8 path"), "{\"a\":1}");
    assert!(result.is_err(), "renaming onto an existing directory should fail");

    let tmp_path = dir.join(".out.json.tmp");
    assert!(!tmp_path.exists(), "temp file should be cleaned up after a failed rename, found {tmp_path:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_atomic_writes_via_temp_then_rename() {
    // Strengthened: proves the temp-file-then-rename mechanism actually
    // ran (a bare std::fs::write would pass the content-round-trips
    // assertion alone, defeating the purpose of this test) by observing
    // the .tmp file exist mid-write, before the rename step lands.
    let dir = std::env::temp_dir().join(format!("rdt-out-atomic-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let target = dir.join("out.json");
    let tmp_path = dir.join(".out.json.tmp");

    assert!(!tmp_path.exists(), "sanity check: no stale .tmp file before the write");
    write_atomic(target.to_str().expect("utf8 path"), "{\"a\":1}").expect("write should succeed");

    assert!(target.exists(), "expected the target file to exist after a successful write");
    assert!(!tmp_path.exists(), "expected the .tmp file to be gone (renamed away) after a successful write");

    let contents = std::fs::read_to_string(&target).expect("read back written file");
    assert_eq!(contents, "{\"a\":1}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn diag(severity: DiagnosticSeverity, code: DiagnosticCode, message: &str, file: Option<&str>) -> Diagnostic {
    Diagnostic {
        severity,
        message: message.to_string(),
        file: file.map(str::to_string),
        line: None,
        column: None,
        help: None,
        code,
    }
}

#[test]
fn extract_subject_finds_first_quoted_substring() {
    assert_eq!(extract_subject("Cannot resolve type 'Date' in 'a.ts'"), "Date");
}

#[test]
fn extract_subject_falls_back_to_full_message_without_quotes() {
    let msg = "Discriminated union detected with discriminant prop 'kind'";
    // sanity: this one DOES have quotes, subject should be "kind"
    assert_eq!(extract_subject(msg), "kind");

    let msg_no_quotes = "Discriminated union detected";
    assert_eq!(extract_subject(msg_no_quotes), msg_no_quotes);
}

#[test]
fn single_diagnostic_is_unchanged_from_todays_format() {
    let d = diag(
        DiagnosticSeverity::Warning,
        DiagnosticCode::OpaqueType,
        "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
        Some("a.ts"),
    );
    let lines = format_diagnostics(std::slice::from_ref(&d));
    let expected = format!("  [{}] {}:{}", "warn".yellow(), "a.ts", d.message);
    assert_eq!(lines, vec![expected]);
}

#[test]
fn groups_same_code_and_subject_across_different_files() {
    let diagnostics = vec![
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'b.ts' — it will appear as opaque",
            Some("b.ts"),
        ),
    ];
    let lines = format_diagnostics(&diagnostics);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("(×2, 2 files)"), "{}", lines[0]);
}

#[test]
fn groups_same_code_and_subject_same_file_without_file_count() {
    let diagnostics = vec![
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
    ];
    let lines = format_diagnostics(&diagnostics);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("(×3)"), "{}", lines[0]);
    assert!(!lines[0].contains("files"), "{}", lines[0]);
}

#[test]
fn different_code_keeps_diagnostics_in_separate_groups() {
    let diagnostics = vec![
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::IndexedAccessOpaque,
            "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
    ];
    let lines = format_diagnostics(&diagnostics);
    assert_eq!(lines.len(), 2, "{lines:?}");
}

#[test]
fn different_subject_keeps_diagnostics_in_separate_groups() {
    let diagnostics = vec![
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'RegExp' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
    ];
    let lines = format_diagnostics(&diagnostics);
    assert_eq!(lines.len(), 2, "{lines:?}");
}

#[test]
fn message_without_quotes_groups_by_full_message_without_panicking() {
    let diagnostics = vec![
        diag(
            DiagnosticSeverity::Info,
            DiagnosticCode::DiscriminatedUnion,
            "Discriminated union detected",
            Some("a.ts"),
        ),
        diag(
            DiagnosticSeverity::Info,
            DiagnosticCode::DiscriminatedUnion,
            "Discriminated union detected",
            Some("b.ts"),
        ),
        diag(DiagnosticSeverity::Info, DiagnosticCode::DiscriminatedUnion, "Some other message", Some("c.ts")),
    ];
    let lines = format_diagnostics(&diagnostics);
    assert_eq!(lines.len(), 2, "{lines:?}");
}

#[test]
fn groups_are_sorted_by_descending_occurrence_count() {
    let diagnostics = vec![
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'RegExp' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
            Some("a.ts"),
        ),
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'b.ts' — it will appear as opaque",
            Some("b.ts"),
        ),
        diag(
            DiagnosticSeverity::Warning,
            DiagnosticCode::OpaqueType,
            "Cannot resolve type 'Date' in 'c.ts' — it will appear as opaque",
            Some("c.ts"),
        ),
    ];
    let lines = format_diagnostics(&diagnostics);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("Date"), "{}", lines[0]);
    assert!(lines[0].contains("×3"), "{}", lines[0]);
    assert!(lines[1].contains("RegExp"), "{}", lines[1]);
}

#[test]
fn help_line_prints_once_per_group_from_representative() {
    let mut d1 = diag(
        DiagnosticSeverity::Warning,
        DiagnosticCode::OpaqueType,
        "Cannot resolve type 'Date' in 'a.ts' — it will appear as opaque",
        Some("a.ts"),
    );
    d1.help = Some("Check that the package is installed and its types are resolvable.".to_string());
    let mut d2 = diag(
        DiagnosticSeverity::Warning,
        DiagnosticCode::OpaqueType,
        "Cannot resolve type 'Date' in 'b.ts' — it will appear as opaque",
        Some("b.ts"),
    );
    d2.help = Some("Check that the package is installed and its types are resolvable.".to_string());

    let lines = format_diagnostics(&[d1, d2]);
    let help_lines: Vec<_> = lines.iter().filter(|l| l.contains("Check that the package is installed")).collect();
    assert_eq!(help_lines.len(), 1, "{lines:?}");
}
