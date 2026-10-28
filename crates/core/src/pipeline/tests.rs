use super::discover::discover_files;
use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn html_attribute_mode_parses_its_three_names_and_rejects_anything_else() {
    assert_eq!(HtmlAttributeMode::parse("curated"), Ok(HtmlAttributeMode::Curated));
    assert_eq!(HtmlAttributeMode::parse("full"), Ok(HtmlAttributeMode::Full));
    assert_eq!(HtmlAttributeMode::parse("none"), Ok(HtmlAttributeMode::None));
    for bad in ["all", "Full", "", "curated "] {
        assert_eq!(HtmlAttributeMode::parse(bad), Err(bad));
    }
}

#[test]
fn merge_cached_dts_file_does_not_drop_parse_diagnostics() {
    // Adversarial review finding: Phase 3.5 (@types/react Full-mode merge)
    // and Phase 3.6 (ambient lib.d.ts merge) each called global.merge()
    // directly on freshly-parsed data with no diagnostics drain first —
    // GlobalSourceData has no diagnostics field, so anything the extractor
    // flagged while parsing either file (an excessive-nesting trip, a
    // parse error) was silently lost. Mirrors
    // extractor::tests::test_excessive_nesting_guard's exact repro shape.
    let tmp = TempDir::new().unwrap();
    let nested = "(".repeat(2500) + &")".repeat(2500);
    let source = format!("const x = {nested};");
    let path = write_file(&tmp, "lib.d.ts", &source);

    let cache_dir = Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap();
    let cache = DtsCache::load_from_disk(Some(&cache_dir));
    let mut global = GlobalSourceData::default();
    let mut diagnostics = Vec::new();

    merge_cached_dts_file(&path, &cache, &mut global, &mut diagnostics);

    assert!(
        diagnostics.iter().any(|d| d.code == DiagnosticCode::ExcessiveNesting),
        "expected the excessive-nesting diagnostic to survive the merge, got {:?}",
        diagnostics
    );
}

#[test]
fn the_default_cache_dir_is_under_the_project_the_source_directory_is_in() {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    write_file(&tmp, "package.json", "{}");
    fs::create_dir_all(tmp.path().join("packages/ui/src")).unwrap();
    let options = PipelineOptions { src_dirs: vec![root.join("packages/ui/src")], ..Default::default() };

    // The nearest ancestor with a `package.json` or a `node_modules`, wherever the process runs.
    assert_eq!(options.effective_cache_dir(), root.join("node_modules/.cache/caliper-docgen"));

    // A `node_modules` nearer the source directory is the project's root.
    fs::create_dir_all(tmp.path().join("packages/ui/node_modules")).unwrap();
    assert_eq!(options.effective_cache_dir(), root.join("packages/ui/node_modules/.cache/caliper-docgen"));

    // An explicit directory always wins.
    let explicit = PipelineOptions { cache_dir: Some(root.join("elsewhere")), ..options };
    assert_eq!(explicit.effective_cache_dir(), root.join("elsewhere"));
}

#[test]
fn parsing_a_dts_file_apart_from_merging_it_gives_what_merging_it_in_one_step_gives() {
    // A cold run parses the lib files while it parses the project's, then merges them in order: the split must lose
    // nothing the one-step merge keeps, diagnostics included.
    let tmp = TempDir::new().unwrap();
    let nested = "(".repeat(2500) + &")".repeat(2500);
    let cache_dir = Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap();
    let cache = DtsCache::load_from_disk(Some(&cache_dir));

    for (name, source, interfaces, nesting_reported) in [
        ("clean.d.ts", "export interface Declared { a: string }".to_owned(), 1, false),
        ("nested.d.ts", format!("const x = {nested};"), 0, true),
    ] {
        let path = write_file(&tmp, name, &source);

        let mut one_step = GlobalSourceData::default();
        let mut one_step_diagnostics = Vec::new();
        merge_cached_dts_file(&path, &cache, &mut one_step, &mut one_step_diagnostics);

        let mut split = GlobalSourceData::default();
        let mut split_diagnostics = Vec::new();
        merge_parsed_dts_file(&path, parse_cached_dts_file(&path, &cache), &mut split, &mut split_diagnostics);

        assert_eq!(
            split.interfaces.keys().collect::<Vec<_>>(),
            one_step.interfaces.keys().collect::<Vec<_>>(),
            "{name}"
        );
        assert_eq!(split.interfaces.len(), interfaces, "{name}");
        assert_eq!(split_diagnostics, one_step_diagnostics, "{name}");
        assert_eq!(
            split_diagnostics.iter().any(|d| d.code == DiagnosticCode::ExcessiveNesting),
            nesting_reported,
            "{name}"
        );
    }
}

#[test]
fn an_unreadable_dts_file_is_reported_once_and_merges_nothing() {
    let tmp = TempDir::new().unwrap();
    let missing = Utf8PathBuf::from_path_buf(tmp.path().join("absent.d.ts")).unwrap();
    let cache_dir = Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap();
    let cache = DtsCache::load_from_disk(Some(&cache_dir));

    let parsed = parse_cached_dts_file(&missing, &cache);
    assert!(parsed.is_err(), "an unreadable file is not parsed as empty and cached");

    let mut global = GlobalSourceData::default();
    let mut diagnostics = Vec::new();
    merge_parsed_dts_file(&missing, parsed, &mut global, &mut diagnostics);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::IoError);
    assert!(global.interfaces.is_empty());
}

/// Write a file into a temp dir and return its Utf8PathBuf.
fn write_file(dir: &TempDir, name: &str, content: &str) -> Utf8PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    Utf8PathBuf::from_path_buf(path).unwrap()
}

// ── a_panic_during_parse_phase_degrades_to_a_diagnostic_not_a_crash ──────
//
// No plugin system exists in this codebase yet to inject a panic via a
// hook (that's a separate, later task), so this drives the real Phase 2
// rayon closure via the sentinel filename it checks for under
// `#[cfg(test)]` (see the `PARSE_PANIC_TEST_SENTINEL` check above).

#[test]
fn a_panic_during_parse_phase_degrades_to_a_diagnostic_not_a_crash() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");
    write_file(&tmp, PARSE_PANIC_TEST_SENTINEL, "export const Other = () => null;");

    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    // Must not panic the test process — that's the whole point.
    let output = extract(&options);

    assert_eq!(output.stats.files_parsed, 2, "both files should still be counted as discovered");
    assert!(
        output.diagnostics.iter().any(|d| d.code == DiagnosticCode::InternalPanic),
        "expected an InternalPanic diagnostic, got {:?}",
        output.diagnostics
    );
}

// ── a_panic_during_resolve_phase_degrades_to_a_diagnostic_not_a_crash ────
//
// Same rationale as the parse-phase test above: `resolve_component` can't
// be made to panic without editing non-test resolver code, so this drives
// the real Phase 4 rayon closure via the sentinel component name it
// checks for under `#[cfg(test)]`.

#[test]
fn a_panic_during_resolve_phase_degrades_to_a_diagnostic_not_a_crash() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");
    write_file(
        &tmp,
        "Other.tsx",
        &format!("export function {RESOLVE_PANIC_TEST_SENTINEL}(props: {{ label: string }}) {{ return null; }}\n"),
    );

    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    // Must not panic the test process — that's the whole point.
    let output = extract(&options);

    assert!(
        output.components.contains_key("Button"),
        "the component whose sibling panicked during resolution should still be extracted"
    );
    assert!(
        output.diagnostics.iter().any(|d| d.code == DiagnosticCode::InternalPanic),
        "expected an InternalPanic diagnostic, got {:?}",
        output.diagnostics
    );
}

// ── test_discover_files ───────────────────────────────────────────────────

#[test]
fn test_discover_files() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");
    write_file(&tmp, "types.ts", "export type Foo = string;");
    write_file(&tmp, "styles.css", ".btn {}");
    write_file(&tmp, "README.md", "# docs");

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, _diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"), "should find .tsx files");
    assert!(names.contains(&"types.ts"), "should find .ts files");
    assert!(!names.contains(&"styles.css"), "should skip .css files");
    assert!(!names.contains(&"README.md"), "should skip .md files");
}

// ── test_exclude_stories ──────────────────────────────────────────────────

#[test]
fn test_exclude_stories() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");
    write_file(&tmp, "Button.stories.tsx", "export default {};");
    write_file(&tmp, "Button.test.tsx", "it('works', () => {});");
    write_file(&tmp, "Button.spec.ts", "");

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, _diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"), "implementation should be included");
    assert!(!names.contains(&"Button.stories.tsx"), ".stories. should be excluded");
    assert!(!names.contains(&"Button.test.tsx"), ".test. should be excluded");
    assert!(!names.contains(&"Button.spec.ts"), ".spec. should be excluded");
}

// ── test_discover_files_reports_diagnostic_for_permission_denied_subtree ─
//
// Bug A (root-cause-analysis.md): `discover_files` used `walker.flatten()`,
// silently dropping every `ignore::Walk` `Err` — a permission-denied
// subtree, a broken symlink, or any other I/O error mid-walk vanished with
// no diagnostic, no warning, nothing. This exercises the permission-denied
// case with a real unreadable subdirectory.

#[test]
#[cfg(unix)]
fn test_discover_files_reports_diagnostic_for_permission_denied_subtree() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let restricted = tmp.path().join("restricted");
    fs::create_dir(&restricted).unwrap();
    fs::write(restricted.join("Hidden.tsx"), "export const Hidden = () => null;").unwrap();
    write_file(&tmp, "Visible.tsx", "export const Visible = () => null;");

    fs::set_permissions(&restricted, fs::Permissions::from_mode(0o000)).unwrap();

    // Root bypasses Unix DAC permission bits entirely, so under a root
    // test runner the 0o000 directory stays readable and this test's
    // premise doesn't hold. Detect that behaviorally (no libc dependency
    // needed) and skip rather than fail spuriously.
    if fs::read_dir(&restricted).is_ok() {
        eprintln!(
            "skipping test_discover_files_reports_diagnostic_for_permission_denied_subtree: \
             running as root, which bypasses Unix permission bits"
        );
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, diagnostics) = discover_files(&[dir], &[]);

    // Restore permissions so TempDir's Drop can actually remove the directory.
    fs::set_permissions(&restricted, fs::Permissions::from_mode(0o755)).unwrap();

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Visible.tsx"), "readable file should still be discovered");
    assert!(!names.contains(&"Hidden.tsx"), "unreadable subtree's file must not silently appear");
    assert!(
        diagnostics.iter().any(|d| d.code == DiagnosticCode::IoError),
        "expected an IoError diagnostic for the unreadable subtree, got {:?}",
        diagnostics
    );
}

// ── test_discover_files_reports_diagnostic_for_non_utf8_filename ─────────
//
// Bug A, second half: a non-UTF8 filename made `Utf8PathBuf::from_path_buf`
// fail, and the `if let Ok(utf8) = ...` branch had no `else` — the file
// silently vanished from the discovered set with zero diagnostic.

#[test]
#[cfg(unix)]
fn test_discover_files_reports_diagnostic_for_non_utf8_filename() {
    use std::os::unix::ffi::OsStrExt;

    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");

    // 0xFF is invalid UTF-8 in any position — construct a non-UTF8 filename
    // directly, bypassing Rust's &str API (which can't represent one).
    let bad_name = std::ffi::OsStr::from_bytes(b"Bad\xFF.tsx");

    // ext4 and friends store filenames as opaque bytes, so this write
    // succeeds on Linux. APFS (macOS) and NTFS (Windows) validate
    // UTF-8/UTF-16 at the syscall level and reject it outright — there is
    // no way to get a non-UTF8 path onto disk there at all, Rust API or
    // not. Detect that behaviorally and skip rather than failing on the
    // filesystem's precondition instead of the code under test.
    if let Err(err) = fs::write(tmp.path().join(bad_name), "export const Bad = () => null;") {
        eprintln!(
            "skipping test_discover_files_reports_diagnostic_for_non_utf8_filename: \
             this filesystem rejects non-UTF8 filenames outright ({err})"
        );
        return;
    }

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"), "valid-UTF8 file should still be discovered");
    assert_eq!(files.len(), 1, "non-UTF8 filename must not silently appear in the discovered set");
    assert!(
        diagnostics.iter().any(|d| d.code == DiagnosticCode::IoError),
        "expected an IoError diagnostic for the non-UTF8 filename, got {:?}",
        diagnostics
    );
}

// ── test_extract_empty_src ────────────────────────────────────────────────

#[test]
fn test_extract_empty_src() {
    let tmp = TempDir::new().unwrap();
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();

    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    assert!(output.components.is_empty(), "no components from empty dir");
    assert!(output.enums.is_empty(), "no enums from empty dir");
    assert!(output.diagnostics.is_empty(), "no diagnostics from empty dir");
    assert_eq!(output.stats.files_parsed, 0);
    assert_eq!(output.stats.components_extracted, 0);
}

// ── test_extract_missing_src_dir ──────────────────────────────────────────

#[test]
fn test_extract_missing_src_dir() {
    // A typo'd/stale --src must surface as an Error diagnostic, not silently
    // produce an empty-but-valid-looking result (AGENTS.md non-negotiable #6).
    let tmp = TempDir::new().unwrap();
    let missing = Utf8PathBuf::from_path_buf(tmp.path().join("does-not-exist")).unwrap();

    let options = PipelineOptions {
        src_dirs: vec![missing.clone()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let error = output
        .diagnostics
        .iter()
        .find(|d| matches!(d.severity, DiagnosticSeverity::Error) && d.code == DiagnosticCode::IoError)
        .expect("missing src dir should produce an Error/IoError diagnostic");
    assert!(error.message.contains(missing.as_str()), "message should name the missing path");
}

// ── test_extract_empty_src_dirs_produces_diagnostic ───────────────────────
//
// Bug B (root-cause-analysis.md): the guard
// `!options.src_dirs.is_empty() && missing_src_dirs.len() == options.src_dirs.len()`
// short-circuits to `false` when `src_dirs` itself is empty (`!true` is
// `false`), so an explicitly empty `src_dirs` bypassed both the "all
// missing" diagnostic and the per-dir "missing" loop — a silent zero-file,
// zero-diagnostic run.

#[test]
fn test_extract_empty_src_dirs_produces_diagnostic() {
    let tmp = TempDir::new().unwrap();
    let options = PipelineOptions {
        src_dirs: vec![],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    assert!(output.components.is_empty());
    assert_eq!(output.stats.files_parsed, 0);
    let error = output
        .diagnostics
        .iter()
        .find(|d| matches!(d.severity, DiagnosticSeverity::Error) && d.code == DiagnosticCode::IoError)
        .expect("empty src_dirs should produce an Error/IoError diagnostic, not a silent empty run");
    assert!(
        error.message.to_lowercase().contains("no source director"),
        "expected the diagnostic to explain that no source directories were configured, got: {}",
        error.message
    );
}

// ── a redeclared prop in a child interface wins over the parent's ─────────

#[test]
fn child_interface_redeclaring_a_prop_wins_over_every_ancestor() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "Comp.tsx",
        r#"
interface Base {
  /**
   * base doc
   * @default "base"
   */
  x: string;
  y: number;
}
interface Mid extends Base {
  /** mid doc */
  x: "a" | "b" | "c";
}
interface Leaf extends Mid {
  /** leaf doc */
  x: "a" | "b";
}
interface Quiet extends Base {
  x: "a" | "b";
}
interface Quieter extends Quiet {
  x: "a";
}
export function FromMid(props: Mid) { return null; }
export function FromLeaf(props: Leaf) { return null; }
export function FromQuiet(props: Quiet) { return null; }
export function FromQuieter(props: Quieter) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let literals =
        |members: &[&str]| PropType::Union(members.iter().map(|m| PropType::StringLiteral((*m).into())).collect());
    let x_of = |component: &str| {
        let x = &output.components[component].props["x"];
        (x.prop_type.clone(), x.description.clone(), x.parent.as_ref().map(|p| p.name.clone()))
    };

    assert_eq!(x_of("FromMid"), (literals(&["a", "b", "c"]), "mid doc".to_owned(), Some("Mid".to_owned())));
    assert_eq!(x_of("FromLeaf"), (literals(&["a", "b"]), "leaf doc".to_owned(), Some("Leaf".to_owned())));

    // An undocumented redeclaration inherits the description, transitively, like TS's JSDoc lookup; tags and
    // `@default` stay with the declaration that wrote them.
    assert_eq!(x_of("FromQuiet"), (literals(&["a", "b"]), "base doc".to_owned(), Some("Quiet".to_owned())));
    assert_eq!(
        x_of("FromQuieter"),
        (PropType::StringLiteral("a".into()), "base doc".to_owned(), Some("Quieter".to_owned()))
    );
    let quiet_x = &output.components["FromQuiet"].props["x"];
    assert_eq!((quiet_x.default_value.as_ref(), quiet_x.tags.is_empty()), (None, true));

    let y = &output.components["FromLeaf"].props["y"];
    assert_eq!(y.prop_type, PropType::Number);
    assert_eq!(y.parent.as_ref().map(|p| p.name.as_str()), Some("Base"));
}

// ── a prop two intersection members declare has both types ───────────────

#[test]
fn a_prop_two_intersection_members_declare_has_both_types() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "Field.tsx",
        r#"
interface Native {
  /** native doc */
  onChange?: (event: Event) => void;
  title?: string;
  label?: string;
  size?: "sm" | "md" | null;
  align?: "start";
}
interface Custom {
  onChange?: (value: string) => void;
  title?: string;
  label: string;
  size?: "md" | "lg";
  align?: "end";
}
interface Extended extends Native { onChange?: (value: string) => void }
type FieldProps = Native & Custom;
export function Field(props: Native & Custom) { return null; }
export function Aliased(props: FieldProps) { return null; }
export function Flipped(props: Custom & Native) { return null; }
export function Child(props: Extended) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let props = |component: &str| {
        output.components[component]
            .props
            .values()
            .map(|p| {
                let parent = p.parent.as_ref().map(|parent| parent.name.clone());
                (p.name.clone(), p.prop_type.raw_string(), p.required, p.description.clone(), parent)
            })
            .collect::<Vec<_>>()
    };
    let prop = |name: &str, ty: &str, required: bool, description: &str, parent: &str| {
        (name.to_owned(), ty.to_owned(), required, description.to_owned(), Some(parent.to_owned()))
    };

    // Literal types reduce to the values they share, as TypeScript reduces them.
    let field = [
        prop("align", "never", false, "", "Native"),
        prop("label", "string", true, "", "Native"),
        prop("onChange", "((event: Event) => void) & ((value: string) => void)", false, "native doc", "Native"),
        prop("size", "\"md\"", false, "", "Native"),
        prop("title", "string", false, "", "Native"),
    ];
    assert_eq!(props("Field"), field);
    assert_eq!(props("Aliased"), field);
    assert_eq!(
        props("Flipped"),
        [
            prop("align", "never", false, "", "Custom"),
            prop("label", "string", true, "", "Custom"),
            prop("onChange", "((value: string) => void) & ((event: Event) => void)", false, "native doc", "Custom"),
            prop("size", "\"md\"", false, "", "Custom"),
            prop("title", "string", false, "", "Custom"),
        ]
    );
    // An interface's own declaration replaces the one it extends instead.
    assert_eq!(
        props("Child"),
        [
            prop("align", "\"start\"", false, "", "Native"),
            prop("label", "string", false, "", "Native"),
            prop("onChange", "(value: string) => void", false, "native doc", "Extended"),
            prop("size", "\"sm\" | \"md\" | null", false, "", "Native"),
            prop("title", "string", false, "", "Native"),
        ]
    );
}

struct RedescribeButtonTone;

impl crate::plugin::DocgenPlugin for RedescribeButtonTone {
    fn name(&self) -> &str {
        "redescribe-button-tone"
    }

    fn on_component_resolved(&self, entry: &mut ComponentEntry) {
        if entry.display_name == "Button" {
            if let Some(tone) = entry.props.get_mut("tone") {
                std::sync::Arc::make_mut(tone).description = "Button's tone".into();
            }
        }
    }
}

#[test]
fn a_plugin_changing_an_inherited_prop_changes_it_for_that_component_only() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "Comp.tsx",
        r#"
interface Base { /** shared tone */ tone?: string }
interface ButtonProps extends Base { a?: string }
interface ChipProps extends Base { b?: string }
export function Button(props: ButtonProps) { return null; }
export function Chip(props: ChipProps) { return null; }
"#,
    );
    let mut options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    options.plugins.register(RedescribeButtonTone);
    let output = extract(&options);

    let description = |component: &str| output.components[component].props["tone"].description.as_str();
    assert_eq!((description("Button"), description("Chip")), ("Button's tone", "shared tone"));
}

// ── literal-union alias props: bare members, never pre-quoted ────────────

#[test]
fn literal_union_alias_props_keep_bare_members_through_resolution_and_templates() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "Chip.tsx",
        r#"
type Size = "sm" | "md";
interface ChipProps {
  size?: Size;
  density?: `compact-${Size}`;
}
export function Chip(props: ChipProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);
    let chip = output.components.get("Chip").expect("Chip component not found");

    let size = &chip.props["size"].prop_type;
    assert_eq!(*size, PropType::LiteralUnion { members: vec!["sm".into(), "md".into()], has_default: false });
    assert_eq!(size.raw_string(), r#""sm" | "md""#);

    let density = &chip.props["density"].prop_type;
    assert_eq!(
        *density,
        PropType::LiteralUnion { members: vec!["compact-sm".into(), "compact-md".into()], has_default: false }
    );
}

// ── ambient files contribute types, not components ───────────────────────

/// Full mode merges the real `@types/react` (which declares `class PureComponent<P, S, SS>
/// extends Component<P, S, SS>`). Its component mappings must not leak into the output.
fn full_mode_options_for(tmp: &TempDir) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    }
}

const BUTTON_SOURCE: &str = r#"
import * as React from "react";
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary";
}
export function Button(props: ButtonProps) { return null; }
"#;

#[test]
fn full_mode_does_not_emit_components_declared_in_merged_react_types() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(&tmp, "Button.tsx", BUTTON_SOURCE);

    let output = extract(&full_mode_options_for(&tmp));

    // `TrustedHTML` and friends are a separate, documented same-namespace-resolution gap;
    // the ghost's own symptom is the unresolvable props parameter `P`.
    let ghost_diagnostics: Vec<&Diagnostic> =
        output.diagnostics.iter().filter(|d| d.message.contains("Cannot resolve type 'P'")).collect();
    assert!(ghost_diagnostics.is_empty(), "unexpected diagnostics from the ghost component: {ghost_diagnostics:?}");

    let names: Vec<&str> = output.components.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["Button"]);
}

#[test]
fn full_mode_binds_the_element_in_inherited_react_event_handlers() {
    // `SVGProps<SVGSVGElement>` reaches `DOMAttributes<T>` through `SVGAttributes<T>`; its handlers take the element.
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "Icon.tsx",
        r#"
import * as React from "react";
interface IconProps extends React.SVGProps<SVGSVGElement> { label?: string }
export function Icon(props: IconProps) { return null; }
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> { tone?: string }
export function Button(props: ButtonProps) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    };
    let output = extract(&options);

    let handler = |component: &str, prop: &str| output.components[component].props[prop].prop_type.raw_string();
    assert_eq!(handler("Icon", "onClick"), "(event: MouseEvent<SVGSVGElement>) => void");
    assert_eq!(handler("Icon", "onCopy"), "(event: ClipboardEvent<SVGSVGElement>) => void");
    assert_eq!(handler("Button", "onClick"), "(event: MouseEvent<HTMLButtonElement>) => void");
}

// ── test_html_attribute_mode_full_resolves_real_button_attrs_end_to_end ───

#[test]
fn test_html_attribute_mode_full_resolves_real_button_attrs_end_to_end() {
    // Real end-to-end proof, not synthetic GlobalSourceData: Full mode should
    // actually find and parse this repo's real @types/react and merge a real
    // ButtonHTMLAttributes field into a real component's props. Placed inside
    // the crate dir (not a bare system tempdir) so ancestor-walking node_modules
    // resolution reaches this repo's real, installed @types/react.
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(
        &tmp,
        "Button.tsx",
        r#"
import * as React from "react";
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary";
}
export function Button(props: ButtonProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    };

    let output = extract(&options);

    // @types/react itself was found and parsed — this must never fail with
    // "could not be resolved" (that's the graceful fallback for @types/react
    // genuinely not being installed, which isn't the case here).
    let unresolvable_react =
        output.diagnostics.iter().find(|d| d.message.contains("@types/react could not be resolved"));
    assert!(unresolvable_react.is_none(), "expected @types/react to resolve in this repo's real node_modules");

    let button = output.components.get("Button").expect("Button component not found");
    // A handful of individual cross-referenced fields inside @types/react's own
    // interface chain (AriaAttributes referenced bare from within the same
    // `declare namespace React {}` block, not through an explicit `React.`
    // qualifier) don't resolve yet — a narrower, separate gap in same-namespace
    // sibling reference resolution, not a regression of this feature. The load-
    // bearing claim is that the bulk of a real element's real attributes merge
    // in as genuine own props, matching RDT's flat behavior.
    assert!(
        button.props.len() > 200,
        "expected the bulk of ButtonHTMLAttributes' real ~235 fields to resolve, got {} props",
        button.props.len()
    );
    assert!(
        button.props.contains_key("formAction"),
        "expected a real ButtonHTMLAttributes field in Button's own props, got {:?}",
        button.props.keys().collect::<Vec<_>>()
    );
    assert!(button.props.contains_key("variant"), "own prop 'variant' should still be present");
}

/// `ComponentProps<"div">` lists none of the div's attributes, but TypeScript still intersects one another member
/// declares. Curated mode reads React's declarations for this lookup too.
#[test]
fn a_prop_beside_an_html_layer_also_has_the_elements_attribute_type_in_full_and_curated_mode() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "Legend.tsx",
        r#"
import * as React from "react";
interface LegendProps { onTimeUpdate?: (data: unknown, index: number) => void; payload?: string[] }
export function Legend(props: React.ComponentProps<"div"> & LegendProps) { return null; }
export function Flipped(props: LegendProps & React.ComponentProps<"div">) { return null; }
export function Omitted(props: Omit<React.ComponentProps<"div">, "onTimeUpdate"> & LegendProps) { return null; }
"#,
    );
    let full = extract(&full_mode_options_for(&tmp));
    let curated =
        extract(&PipelineOptions { html_attributes: HtmlAttributeMode::Curated, ..full_mode_options_for(&tmp) });

    let on_time_update = |output: &crate::types::ExtractionOutput, component: &str| {
        let prop = &output.components[component].props["onTimeUpdate"];
        (prop.prop_type.raw_string(), prop.declarations.iter().map(|p| p.name.clone()).collect::<Vec<_>>())
    };
    let dom = "(event: SyntheticEvent<HTMLDivElement>) => void";
    let legend = "(data: unknown, index: number) => void";
    let declared = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        on_time_update(&full, "Legend"),
        (format!("({dom}) & ({legend})"), declared(&["DOMAttributes", "LegendProps"]))
    );
    assert_eq!(
        on_time_update(&full, "Flipped"),
        (format!("({legend}) & ({dom})"), declared(&["LegendProps", "DOMAttributes"]))
    );
    assert_eq!(on_time_update(&full, "Omitted"), (legend.to_owned(), declared(&["LegendProps"])));
    // Curated mode resolves nothing else from React's declarations, so the attribute keeps its alias name.
    let dom = "ReactEventHandler<HTMLDivElement>";
    assert_eq!(
        on_time_update(&curated, "Legend"),
        (format!("{dom} & ({legend})"), declared(&["DOMAttributes", "LegendProps"]))
    );
    assert_eq!(
        on_time_update(&curated, "Flipped"),
        (format!("({legend}) & {dom}"), declared(&["LegendProps", "DOMAttributes"]))
    );
    assert_eq!(on_time_update(&curated, "Omitted"), (legend.to_owned(), declared(&["LegendProps"])));
}

// ── test_generic_svg_attributes_extends_resolves_to_its_element_in_full_mode ─
//
// `IconProps extends React.SVGAttributes<SVGSVGElement>` was resolved as an
// opaque no-op (react_types::html_element_for("SVGAttributes") returns None
// — "no single element to pick", true for the *unparameterized* name but
// not for a real call site, which always supplies a concrete element type
// argument). classify_extends now derives the element from that argument
// for the generic SVGAttributes/SVGProps/HTMLProps forms specifically,
// the same way the concrete `<Element>HTMLAttributes` forms already do
// from their name alone.

#[test]
fn test_generic_svg_attributes_extends_resolves_to_its_element_in_full_mode() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(
        &tmp,
        "Icon.tsx",
        r#"
import * as React from "react";
export interface IconProps extends React.SVGAttributes<SVGSVGElement> {
  size?: number;
}
export const Icon = React.forwardRef<SVGSVGElement, IconProps>((props, ref) => <svg ref={ref} {...props} />);
Icon.displayName = "Icon";
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    };

    let output = extract(&options);

    let icon = output.components.get("Icon").expect("Icon component not found");
    assert!(
        icon.props.contains_key("suppressHydrationWarning"),
        "expected a real SVGAttributes field to resolve as an own prop in Full mode, got {:?}",
        icon.props.keys().collect::<Vec<_>>()
    );
    assert!(icon.props.contains_key("size"), "own prop 'size' should still be present");
}

// ── test_unresolvable_intersection_member_records_raw_type_in_composes ────
//
// `resolve_base_as_chain`'s final match arm (conditional types, mapped types,
// indexed access, etc. used directly as a props base) already builds a real
// diagnostic naming the exact unresolvable expression — but returned a bare
// `ResolvedChain::default()` instead of `empty_with_compose`, so the props
// map ends up with zero trace of it. `composes` (`ComponentEntry.composes:
// Vec<String>`) is the react-docgen-native mechanism for exactly this case
// ("props come from this type, which we're listing by name/expression
// instead of flattening") — populating it here needs no new type inference,
// just recording the raw text this code path was already computing for the
// diagnostic message.

#[test]
fn test_unresolvable_intersection_member_records_raw_type_in_composes() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(
        &tmp,
        "Comp.tsx",
        r#"
export type Weird<T> = T extends { kind: infer K } ? { a: K } : { b: number };
export type CompProps = Weird<'x'> & { c: boolean };
export function Comp(props: CompProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let comp = output.components.get("Comp").expect("Comp component not found");
    assert!(comp.props.contains_key("c"), "expected the resolvable intersection member's prop to survive");
    assert!(
        !comp.composes.is_empty(),
        "expected the unresolvable Weird<'x'> member to be recorded in `composes` instead of silently vanishing"
    );
    // An `infer` needs a checker, so the conditional stays unevaluated.
    assert_eq!(comp.composes, ["\"x\" extends { ... } ? { ... } : { ... }"]);
}

// ── test_same_display_name_across_files_with_identical_stem_does_not_collide ─
//
// Phase 5's dedup key was `"{name} ({file_stem})"` for the 2nd+ occurrence
// of a display name — but file_stem alone isn't unique across different
// src_dirs: two different libraries each shipping a `Button.tsx`/`Button.d.ts`
// (extremely common — found for real across 5 of this repo's own fixture
// libraries: chakra, fluentui, mantine, mui, panda, all named literally
// "Button") produce the identical disambiguation key too, so the 3rd+
// colliding component silently overwrote the 2nd in the output BTreeMap
// with no diagnostic at all — a real violation of "never fail silently"
// for anyone pointing this tool at more than one directory in one call
// (the default and only way `apps/validate` and any real monorepo build
// would invoke it).

#[test]
fn test_same_display_name_across_files_with_identical_stem_does_not_collide() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib_a = TempDir::new_in(manifest_dir).unwrap();
    let lib_b = TempDir::new_in(manifest_dir).unwrap();
    let lib_c = TempDir::new_in(manifest_dir).unwrap();
    write_file(&lib_a, "Button.tsx", "export function Button(props: { a?: string }) { return null; }\n");
    write_file(&lib_b, "Button.tsx", "export function Button(props: { b?: string }) { return null; }\n");
    write_file(&lib_c, "Button.tsx", "export function Button(props: { c?: string }) { return null; }\n");

    let options = PipelineOptions {
        src_dirs: vec![
            Utf8PathBuf::from_path_buf(lib_a.path().to_owned()).unwrap(),
            Utf8PathBuf::from_path_buf(lib_b.path().to_owned()).unwrap(),
            Utf8PathBuf::from_path_buf(lib_c.path().to_owned()).unwrap(),
        ],
        cache_dir: Some(Utf8PathBuf::from_path_buf(lib_a.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    assert_eq!(
        output.components.len(),
        3,
        "expected all 3 same-named, same-stem components to survive under distinct keys, got {:?}",
        output.components.keys().collect::<Vec<_>>()
    );
    let all_props: std::collections::BTreeSet<&str> =
        output.components.values().flat_map(|c| c.props.keys().map(String::as_str)).collect();
    assert_eq!(
        all_props,
        std::collections::BTreeSet::from(["a", "b", "c"]),
        "expected each distinct component's own prop to survive, got {:?}",
        all_props
    );
}

// ── test_named_type_only_import_from_react_resolves_to_real_dts ──────────
//
// Regression test for: `import type { AriaAttributes } from "react"` used
// directly as an `extends` target failed for two independent reasons (this
// uses AriaAttributes rather than HTMLAttributes specifically because
// "HTMLAttributes" gets special-cased as a synthetic "div" element by
// `react_types::html_element_for`, which routes through an unrelated,
// already-working code path — AriaAttributes has no such shortcut, so it
// actually exercises general named-import resolution):
//   1. "react" resolved to its own `index.js` (react's package.json has no
//      "types"/"typings" field and no "types" export condition — its real
//      type declarations live in the separate `@types/react` package),
//      instead of falling back to `@types/react`.
//   2. Even once the right file is found, `@types/react` declares
//      everything inside `declare namespace React { ... }`, so
//      `AriaAttributes` is keyed as "React.AriaAttributes" there — a bare
//      lookup for "AriaAttributes" (the name as actually imported/used)
//      never matches.

#[test]
fn test_named_type_only_import_from_react_resolves_to_real_dts() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(
        &tmp,
        "Panel.tsx",
        r#"
import type { AriaAttributes } from "react";
interface PanelProps extends AriaAttributes {
  collapsible?: boolean;
}
export function Panel(props: PanelProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    };

    let output = extract(&options);

    let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("Cannot resolve type 'AriaAttributes'"));
    assert!(
        unresolvable.is_none(),
        "expected 'react' to resolve to @types/react's real .d.ts, got diagnostic: {:?}",
        unresolvable
    );

    let panel = output.components.get("Panel").expect("Panel component not found");
    assert!(
        panel.props.contains_key("aria-label"),
        "expected a real AriaAttributes field in Panel's own props, got {:?}",
        panel.props.keys().collect::<Vec<_>>()
    );
    assert!(panel.props.contains_key("collapsible"), "own prop 'collapsible' should still be present");
}

// ── test_namespace_import_member_access_resolves_to_real_dts ─────────────
//
// Regression test for: `import * as React from "react"` then a plain field
// reference to `React.DependencyList` failed even in `--html-attributes
// full` mode, where @types/react genuinely is parsed. Root cause:
// `resolve_to_canonical` only stripped the "React." prefix for the
// is_react_builtin check (named.rs step 1) — it never routed a `React.X`
// member-expression reference through the "react" import at all, since
// `find_import` only tracks the namespace binding itself ("React", with
// `exported_name == "*"`), not literal dotted names like
// "React.DependencyList". It always fell back to a same-file lookup for
// that literal dotted name, which of course never resolves.

#[test]
fn test_namespace_import_member_access_resolves_to_real_dts() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(
        &tmp,
        "Table.tsx",
        r#"
import * as React from "react";
interface TableProps {
  cellRendererDependencies?: React.DependencyList;
}
export function Table(props: TableProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    };

    let output = extract(&options);

    let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("DependencyList"));
    assert!(
        unresolvable.is_none(),
        "expected React.DependencyList to resolve via the 'react' namespace import, got diagnostic: {:?}",
        unresolvable
    );

    let table = output.components.get("Table").expect("Table component not found");
    assert!(
        table.props.contains_key("cellRendererDependencies"),
        "expected 'cellRendererDependencies' prop, got: {:?}",
        table.props.keys().collect::<Vec<_>>()
    );
}

/// `menu.tsx` declares `RootProps`; `ui.ts` exposes it as the `Menu` namespace the way given; `Item.tsx` uses it through
/// a named import.
fn namespace_reexport_project(ui: &str) -> ExtractionOutput {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "menu.tsx",
        "export interface RootProps { open: boolean }\nexport function Root(props: RootProps) { return <div />; }\n",
    );
    write_file(&tmp, "ui.ts", ui);
    write_file(
        &tmp,
        "Item.tsx",
        "import { Menu as MenuPrimitive } from './ui';\n\
         export function Item(props: MenuPrimitive.RootProps) { return null; }\n",
    );
    extract(&PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    })
}

#[test]
fn a_member_of_a_namespace_reexport_resolves_through_a_named_import() {
    for ui in [
        "export * as Menu from './menu';\n",
        "import * as menu from './menu';\nexport { menu as Menu };\n",
        "export namespace Menu { export interface RootProps { open: boolean } export const Root = () => null; }\n",
    ] {
        let output = namespace_reexport_project(ui);

        let props: Vec<&String> = output.components["Item"].props.keys().collect();
        assert_eq!(props, ["open"], "{ui}{:#?}", output.diagnostics);
    }
}

#[test]
fn a_jsdoc_comment_documents_the_declaration_right_after_it_only() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "badge.tsx",
        r#"/** The badge. */
export function Badge(props: { tone: string; size: number }) { return <span />; }

/** Not a component's doc: it belongs to the constant. */
const placement = { left: "right" };
export function Arrow(props: { side: string }) { return <span>{placement.left}</span>; }

export interface PanelProps {
  /** Hooks for the panel. */
  hooks: { isOpen?: () => boolean };
  /** @deprecated Use `open` instead. */
  visible?: boolean | { keepMounted?: boolean };
}
/** The panel. */
// eslint-disable-next-line
export function Panel(props: PanelProps) { return <div />; }
"#,
    );
    let output = extract(&PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    });

    let description = |component: &str| output.components[component].description.as_str();
    let prop = |component: &str, name: &str| output.components[component].props[name].clone();
    assert_eq!(description("Badge"), "The badge.");
    assert_eq!(prop("Badge", "tone").description, "");
    assert_eq!(description("Arrow"), "");
    assert_eq!(description("Panel"), "The panel.");
    assert_eq!(prop("Panel", "hooks").description, "Hooks for the panel.");
    let PropType::Object(fields) = &prop("Panel", "hooks").prop_type else { panic!("{:?}", prop("Panel", "hooks")) };
    assert_eq!(fields[0].description, "");
    assert_eq!(prop("Panel", "visible").tags.get("deprecated").map(String::as_str), Some("Use `open` instead."));
}

#[test]
fn component_props_of_typeof_a_component_is_that_components_props() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "menu.tsx",
        "export interface RootProps { open: boolean }\nexport function Root(props: RootProps) { return <div />; }\n",
    );
    write_file(&tmp, "ui.ts", "export * as Menu from './menu';\n");
    write_file(
        &tmp,
        "pagination.tsx",
        r#"import * as React from "react";
import type { ComponentProps } from "react";
import { Root } from "./menu";
import { Menu as MenuPrimitive } from "./ui";
type PaginationLinkProps = { isActive?: boolean; href: string };
export function PaginationLink(props: PaginationLinkProps) { return <a />; }
export function PaginationNext(props: React.ComponentProps<typeof PaginationLink>) { return null; }
export function Imported(props: ComponentProps<typeof Root>) { return null; }
export function Namespaced(props: React.ComponentPropsWithoutRef<typeof MenuPrimitive.Root>) { return null; }
"#,
    );
    write_file(
        &tmp,
        "own.tsx",
        "type ComponentProps<T> = { own: T };\nimport { Root } from './menu';\n\
         export function Own(props: ComponentProps<typeof Root>) { return null; }\n",
    );
    let output = extract(&PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    });

    let props = |component: &str| output.components[component].props.keys().cloned().collect::<Vec<_>>();
    assert_eq!(props("PaginationNext"), ["href", "isActive"], "{:#?}", output.diagnostics);
    assert_eq!(props("Imported"), ["open"], "{:#?}", output.diagnostics);
    assert_eq!(props("Namespaced"), ["open"], "{:#?}", output.diagnostics);
    // A project's own `ComponentProps` is that declaration, not React's.
    assert_eq!(props("Own"), ["own"], "{:#?}", output.diagnostics);
}

// ── test_native_js_global_resolves_via_real_typescript_lib_files ─────────
//
// Regression test for: `Date` (and any other native/DOM ambient global —
// never imported, so nothing ever had a reason to resolve it) spuriously
// triggered "Cannot resolve type 'Date' — it will appear as opaque",
// hundreds of times on a real date-picker-shaped fixture. Real TypeScript
// declares these in its own lib.es5.d.ts/lib.dom.d.ts (no export/import at
// all — ambient script context, not a module) — this test proves the
// pipeline actually finds and parses this repo's real `typescript` package
// (not a synthetic stand-in) and resolves `Date` through it, structurally,
// rather than via a hardcoded name list.

#[test]
fn test_native_js_global_resolves_via_real_typescript_lib_files() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(
        &tmp,
        "DatePicker.tsx",
        r#"
interface DatePickerProps {
  selected?: Date;
  onSelect?: (date: Date) => void;
}
export function DatePicker(props: DatePickerProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("'Date'"));
    assert!(
        unresolvable.is_none(),
        "expected 'Date' to resolve via this repo's real typescript lib.es5.d.ts, got diagnostic: {:?}",
        unresolvable
    );

    let picker = output.components.get("DatePicker").expect("DatePicker component not found");
    assert!(
        picker.props.contains_key("selected"),
        "expected 'selected' prop, got: {:?}",
        picker.props.keys().collect::<Vec<_>>()
    );
}

// ── test_indexed_access_into_ambient_dom_interface_resolves_via_real_lib ──
//
// Regression test for: day-picker's `HTMLDivElement["dir"]` (also "nonce",
// "title", "lang") degraded to Opaque even after `lib.dom.d.ts` became
// parseable. Two compounding bugs, both now fixed: (1) `lib.dom.d.ts` was
// silently skipped entirely because `max_bracket_nesting_depth` miscounted
// JSDoc comments containing unmatched brackets (see
// `extractor::tests::test_nesting_guard_ignores_brackets_inside_comments`);
// (2) `dir` isn't declared directly on `HTMLDivElement` — only inherited
// via `extends HTMLElement` — and `resolve_indexed_access` only searched
// an interface's own fields, never its extends chain. This test proves
// both fixes compose correctly against this repo's real installed
// `typescript` package, not just fabricated in-memory data.

#[test]
fn test_indexed_access_into_ambient_dom_interface_resolves_via_real_lib() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(
        &tmp,
        "Foo.tsx",
        r#"
interface FooProps {
  dir?: HTMLDivElement["dir"];
}
export function Foo(props: FooProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("HTMLDivElement"));
    assert!(
        unresolvable.is_none(),
        "expected HTMLDivElement[\"dir\"] to resolve via this repo's real lib.dom.d.ts, got diagnostic: {:?}",
        unresolvable
    );

    let foo = output.components.get("Foo").expect("Foo component not found");
    let dir_prop = foo.props.get("dir").expect("'dir' prop not found");
    assert_eq!(dir_prop.prop_type, PropType::String, "expected String, got {:?}", dir_prop.prop_type);
}

// ── test_named_type_reexported_through_a_barrel_file_resolves ────────────
//
// `import_map.rs`'s `resolve_reexport_chain`/`wildcard_sources_for` were
// fully implemented and unit-tested in isolation, but `resolve_to_canonical`
// never called them — a type imported through a barrel (`export { X } from
// './x'` or `export * from './x'`, ubiquitous in real component libraries'
// index.ts files) resolved to the barrel file itself, which doesn't declare
// the type, so it silently fell through to "Cannot resolve type" instead of
// following the re-export to where the type actually lives.

#[test]
fn test_named_type_reexported_through_a_barrel_file_resolves() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(&tmp, "types.ts", "export interface ButtonProps { label: string; }\n");
    write_file(&tmp, "index.ts", "export type { ButtonProps } from './types';\n");
    write_file(
        &tmp,
        "Button.tsx",
        r#"
import type { ButtonProps } from './index';
export function Button(props: ButtonProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("Cannot resolve type 'ButtonProps'"));
    assert!(
        unresolvable.is_none(),
        "expected ButtonProps to resolve through the barrel's named re-export, got diagnostic: {:?}",
        unresolvable
    );

    let button = output.components.get("Button").expect("Button component not found");
    assert!(
        button.props.contains_key("label"),
        "expected 'label' prop from the re-exported ButtonProps, got {:?}",
        button.props.keys().collect::<Vec<_>>()
    );
}

// ── test_unresolvable_named_type_reports_barrel_redirected_location ─────
//
// resolver/chain.rs's step-6 "Cannot resolve" diagnostic (component prop
// types) already noted where an import redirected to when it differed from
// the naive name/consuming-file; resolver/named.rs's step-7 diagnostic
// (nested/named type-level, not component-prop-level) never got the same
// treatment. `value` here is imported through a wildcard barrel that never
// actually declares it — a real broken/incomplete barrel — so it stays
// unresolvable, but the diagnostic should still say where the import
// redirected to, not just where it was written.
#[test]
fn test_unresolvable_named_type_reports_barrel_redirected_location() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(&tmp, "empty.ts", "export const nothing = 1;\n");
    write_file(&tmp, "barrel.ts", "export * from './empty';\n");
    write_file(
        &tmp,
        "Component.tsx",
        r#"
import type { NeverDeclared } from './barrel';
export function Component(props: { value: Array<NeverDeclared> }) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("Cannot resolve type 'NeverDeclared'"));
    let unresolvable = unresolvable.expect("expected an unresolvable-type diagnostic for NeverDeclared");
    assert!(
        unresolvable.message.contains("(resolved to 'NeverDeclared' in") && unresolvable.message.contains("barrel.ts"),
        "expected the diagnostic to note the barrel-redirected location, got: {}",
        unresolvable.message
    );
}

#[test]
fn test_named_type_reexported_through_a_wildcard_barrel_file_resolves() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    write_file(&tmp, "types.ts", "export interface ButtonProps { label: string; }\n");
    write_file(&tmp, "index.ts", "export * from './types';\n");
    write_file(
        &tmp,
        "Button.tsx",
        r#"
import type { ButtonProps } from './index';
export function Button(props: ButtonProps) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("Cannot resolve type 'ButtonProps'"));
    assert!(
        unresolvable.is_none(),
        "expected ButtonProps to resolve through the barrel's wildcard re-export, got diagnostic: {:?}",
        unresolvable
    );

    let button = output.components.get("Button").expect("Button component not found");
    assert!(
        button.props.contains_key("label"),
        "expected 'label' prop from the re-exported ButtonProps, got {:?}",
        button.props.keys().collect::<Vec<_>>()
    );
}

// ── test_barrel_resolution_does_not_exponentially_blow_up_on_shared_descendants ──
//
// Adversarial-review finding: follow_reexports (resolver/import.rs) had no
// visited-set, only the MAX_REEXPORT_DEPTH cap. That bounds the RECURSION
// depth but not the number of CALLS — a barrel graph where multiple parents
// wildcard-re-export the same shared descendant (ubiquitous in real
// component libraries: several sub-barrels all wildcarding a common
// `types.ts`) re-explores that shared subtree once per path into it,
// multiplying out to branching_factor^depth calls instead of one call per
// graph node. Reproduced empirically in the review with real distinct
// files: 7 barrels took 9.7s wall time vs 156ms at 2.
//
// Wall-clock isn't a reliable signal for a small, fast-running unit test:
// oxc_resolver caches path resolution by (from_dir, specifier), and this
// repo's own per-call work (a couple of hashmap probes) is cheap enough
// that even a few thousand redundant calls stay under a second on this
// machine. Assert on the actual call count instead — deterministic, and
// it's the real mechanism the fix changes.
//
// This barrel wildcard-re-exports the same next sibling twice, deep enough
// to hit MAX_REEXPORT_DEPTH (8), with a name that's never declared
// anywhere (the unhappy path, which forces full traversal instead of an
// early exit on first match). Without the visited-set fix this explores
// 2^8 = 256 calls; with it, exactly 9 (one per graph node, barrel_0..8).
#[test]
fn test_barrel_resolution_does_not_exponentially_blow_up_on_shared_descendants() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();

    const DEPTH: usize = 8;
    for level in 0..DEPTH {
        write_file(
            &tmp,
            &format!("barrel_{level}.ts"),
            &format!("export * from './barrel_{}';\nexport * from './barrel_{}';\n", level + 1, level + 1),
        );
    }
    write_file(&tmp, &format!("barrel_{DEPTH}.ts"), "export const unrelated = 1;\n");
    write_file(
        &tmp,
        "Component.tsx",
        r#"
import type { NeverDeclared } from './barrel_0';
export function Component(props: { value: NeverDeclared }) { return null; }
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    // Canonical form: resolved import paths are canonicalized, so the scope must be too.
    let scope = Utf8PathBuf::from_path_buf(fs::canonicalize(tmp.path()).unwrap()).unwrap();
    crate::resolver::reset_follow_reexports_call_count(&scope);
    let output = extract(&options);
    let calls = crate::resolver::follow_reexports_call_count();

    // Linear in graph size (barrel_0..barrel_8 = 9 nodes) with the
    // visited-set; without it this would be 2^8 = 256. A generous bound
    // (well under 256, comfortably above 9) that still fails hard if the
    // fix regresses back to exponential.
    assert!(
        calls < 50,
        "follow_reexports was called {calls} times on a depth-{DEPTH} shared-descendant \
         barrel graph (expected ~9, linear in graph size) — looks like the visited-set \
         regressed back to exploring branching_factor^depth calls"
    );
    // NeverDeclared genuinely doesn't exist anywhere — still expect the normal
    // "cannot resolve" diagnostic, not a crash or a hang.
    assert!(output.components.contains_key("Component"), "expected Component to still be extracted");
}

// ── test_static_default_props_assignment_reaches_parsed_prop ─────────────
//
// Regression test for: `Button.defaultProps = { size: 'md' }` (deprecated
// but still real — MUI ships it) was never read at all; only destructured
// defaults (`function Button({ size = 'md' })`) populated
// ComponentMapping.param_defaults. Proves the extraction fix reaches all
// the way through to ExtractionOutput, not just SourceData.

#[test]
fn test_static_default_props_assignment_reaches_parsed_prop() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "Button.tsx",
        r#"
interface ButtonProps {
  size?: string;
}
export function Button(props: ButtonProps) { return null; }
Button.defaultProps = { size: 'md' };
"#,
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };

    let output = extract(&options);

    let button = output.components.get("Button").expect("Button component not found");
    let size_prop = button.props.get("size").expect("'size' prop not found");
    let default = size_prop.default_value.as_ref().expect("expected a default value for 'size'");
    assert_eq!(default.value, "'md'");
    assert!(!default.computed);
}

// ── test_pipeline_options_default ─────────────────────────────────────────

#[test]
fn test_pipeline_options_default() {
    let opts = PipelineOptions::default();
    assert_eq!(opts.src_dirs, vec![Utf8PathBuf::from("./src")]);
    assert_eq!(opts.html_attributes, HtmlAttributeMode::Curated);
    assert!(opts.exclude_patterns.is_empty() && opts.exclude_prefixes.is_empty() && opts.extra_builtins.is_empty());
    assert_eq!((opts.tsconfig_path, opts.cache_dir), (None, None));
}

// ── test_watch_session_update ─────────────────────────────────────────────

#[test]
fn test_watch_session_update() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    // Modify the file and trigger an incremental update.
    let button_path = Utf8PathBuf::from_path_buf(tmp.path().join("Button.tsx")).unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = ({ label }: { label: string }) => null;");

    let update = session.update_file(&button_path);

    // With the stub resolver, no components are returned (mappings list may be
    // empty for a simple stub file), but the call must complete without panic.
    assert!(update.duration_ms < 5_000, "update should complete quickly");
    // The changed file itself must appear in affected_files. update_file
    // canonicalizes its input (matching discover_files' own symlink-
    // resolved output, e.g. macOS's /var -> /private/var) so compare
    // against the canonicalized form, not necessarily the raw input.
    let canonical_button_path =
        Utf8PathBuf::from_path_buf(std::fs::canonicalize(button_path.as_std_path()).unwrap()).unwrap();
    assert!(
        update.affected_files.contains(&canonical_button_path),
        "changed file should always appear in affected_files"
    );
}

#[test]
fn test_pipeline_plugin_execution() {
    use crate::plugin::{DocgenPlugin, PluginRegistry};
    use crate::types::ComponentEntry;

    struct TestPlugin;
    impl DocgenPlugin for TestPlugin {
        fn name(&self) -> &str {
            "test-plugin"
        }
        fn on_component_resolved(&self, entry: &mut ComponentEntry) {
            entry.composes.push("PluginAdded".into());
        }
    }

    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");

    let mut plugins = PluginRegistry::new();
    plugins.register(TestPlugin);

    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        plugins,
        ..Default::default()
    };

    let output = extract(&options);
    let button = output.components.get("Button").expect("Button component not found");
    assert_eq!(button.composes, vec!["PluginAdded"]);
}

#[test]
fn a_panicking_plugin_hook_degrades_to_a_diagnostic_and_other_work_still_completes() {
    use crate::plugin::{DocgenPlugin, PluginRegistry};
    use crate::types::ComponentEntry;

    struct AlwaysPanicsPlugin;
    impl DocgenPlugin for AlwaysPanicsPlugin {
        fn name(&self) -> &str {
            "always-panics"
        }
        fn on_component_resolved(&self, _entry: &mut ComponentEntry) {
            panic!("boom");
        }
    }

    struct WellBehavedPlugin;
    impl DocgenPlugin for WellBehavedPlugin {
        fn name(&self) -> &str {
            "well-behaved"
        }
        fn on_component_resolved(&self, entry: &mut ComponentEntry) {
            entry.composes.push("PluginAdded".into());
        }
    }

    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");
    write_file(&tmp, "Card.tsx", "export function Card(props: { title: string }) { return null; }\n");

    let mut plugins = PluginRegistry::new();
    plugins.register(AlwaysPanicsPlugin);
    plugins.register(WellBehavedPlugin);

    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        plugins,
        ..Default::default()
    };

    // Must not panic the test process — that's the whole point.
    let output = extract(&options);

    let panic_diagnostics: Vec<_> =
        output.diagnostics.iter().filter(|d| d.code == DiagnosticCode::InternalPanic).collect();
    // The identical panic from both components is one diagnostic listing both.
    assert_eq!(
        panic_diagnostics.iter().map(|d| d.affects.clone()).collect::<Vec<_>>(),
        vec![vec!["Button".to_string(), "Card".to_string()]],
        "got {:?}",
        output.diagnostics
    );
    assert!(
        panic_diagnostics.iter().all(|d| d.message.contains("always-panics")),
        "each diagnostic should name the panicking plugin, got {panic_diagnostics:?}"
    );

    // Both components must still be present — the panicking plugin's hook
    // failure on one entry must not drop that entry, nor prevent the
    // sibling entry from being processed.
    let button = output.components.get("Button").expect("Button component not found");
    let card = output.components.get("Card").expect("Card component not found");

    // The well-behaved plugin, registered after the panicking one, must
    // still have run for both entries.
    assert_eq!(button.composes, vec!["PluginAdded"]);
    assert_eq!(card.composes, vec!["PluginAdded"]);
}

// ── test_duplicate_component_declarations_in_one_file_emit_a_collision_diagnostic ─
//
// Bug C (root-cause-analysis.md): `components.insert(key, entry)` in Phase
// 5 discarded the `Option<ComponentEntry>` `BTreeMap::insert` already hands
// back — a same-key collision (three or more resolutions landing on the
// identical disambiguated key) silently overwrote the earlier entry with
// zero diagnostic. `parse_file` never runs `oxc_semantic`'s checker, so
// three syntactically-duplicate `function Button(...)` declarations in one
// file parse cleanly into three `ComponentMapping`s sharing both name and
// file_path (see extractor::visit's `visit_function`) — the 1st occurrence
// keys as "Button", the 2nd and 3rd both key as "Button (<path>)" (file_path
// is identical for all three), colliding at insert exactly like an
// overlapping-src_dirs 3-way repeat would. (Note: a *2-directory* overlap of
// the same physical file is instead caught upstream by `discover_files`'s
// post-sort dedup — see `test_overlapping_src_dirs_two_directories_deduplicates_without_a_duplicate_listing` —
// since that case never reaches this collision at all.)

#[test]
fn test_duplicate_component_declarations_in_one_file_emit_a_collision_diagnostic() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "Button.tsx",
        "interface ButtonProps { a?: string }\n\
         export function Button(props: ButtonProps) { return null; }\n\
         export function Button(props: ButtonProps) { return null; }\n\
         export function Button(props: ButtonProps) { return null; }\n",
    );

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let collision = output
        .diagnostics
        .iter()
        .find(|d| d.code == DiagnosticCode::ComponentKeyCollision)
        .expect("expected a ComponentKeyCollision diagnostic, got none");
    assert!(
        collision.message.contains("Button.tsx"),
        "expected the diagnostic to name the colliding file path, got: {}",
        collision.message
    );
}

// A name exported by two files must resolve identically whatever order the source directories are given in.
#[test]
fn test_same_named_exported_enums_resolve_deterministically_and_warn() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("a")).unwrap();
    fs::create_dir_all(tmp.path().join("b")).unwrap();
    write_file(&tmp, "a/Size.ts", "export enum Size { Small = \"sm\" }\n");
    write_file(&tmp, "b/Size.ts", "export enum Size { Large = \"lg\" }\n");

    let run = |first: &str, second: &str| {
        let options = PipelineOptions {
            src_dirs: vec![
                Utf8PathBuf::from_path_buf(tmp.path().join(first)).unwrap(),
                Utf8PathBuf::from_path_buf(tmp.path().join(second)).unwrap(),
            ],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };
        extract(&options)
    };

    let forward = run("a", "b");
    let reversed = run("b", "a");

    assert_eq!(serde_json::to_value(&forward.enums).unwrap(), serde_json::to_value(&reversed.enums).unwrap());
    let sizes: Vec<String> = forward.enums["Size"].iter().map(|e| e.name.to_string()).collect();
    assert_eq!(sizes, vec!["Small"]);
    let dupes = |o: &ExtractionOutput| -> Vec<String> {
        o.diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::DuplicateEnumName)
            .map(|d| d.message.clone())
            .collect()
    };
    assert_eq!(dupes(&forward).len(), 1);
    assert_eq!(dupes(&forward), dupes(&reversed));
}

#[test]
fn test_enum_scoped_keys_split_on_the_last_colon() {
    let mut global = GlobalSourceData::default();
    global.enums.insert(
        "C:\\src\\Size.ts:Size".to_owned(),
        vec![EnumEntry { name: "Small".into(), value: EnumValue::String("sm".into()), description: String::new() }],
    );
    global.re_export_map.insert(
        Utf8PathBuf::from("C:\\src\\Size.ts"),
        vec![LexedExport::LocalDeclaration { name: "Size".into(), is_type_only: false, declared_as: None }],
    );
    let (enums, diagnostics) = collect_public_enums(&global);
    assert_eq!(enums.keys().collect::<Vec<_>>(), vec!["Size"]);
    assert!(diagnostics.is_empty());
}

// An unreadable declaration file must surface as an IoError and leave no empty entry in the parse cache.
#[test]
fn test_unreadable_dts_file_is_reported_not_cached_as_empty() {
    let tmp = TempDir::new().unwrap();
    let missing = Utf8PathBuf::from_path_buf(tmp.path().join("react/index.d.ts")).unwrap();
    let cache = DtsCache::load_from_disk(None);
    let mut global = GlobalSourceData::default();
    let mut diagnostics = Vec::new();

    merge_cached_dts_file(&missing, &cache, &mut global, &mut diagnostics);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::IoError);
    assert_eq!(diagnostics[0].file.as_deref(), Some(missing.as_str()));
    assert!(cache.get(&missing, "").is_none(), "an empty parse must not be cached under the unreadable path");
}

// A wrapper-call guess whose props resolve to nothing is reported, not listed as an empty component.
#[test]
fn test_wrapper_call_components_whose_props_cannot_be_resolved_are_kept_with_the_reason() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "Widgets.tsx",
        "import { wrap } from \"./lib\";\n\
         import type { MissingProps } from \"./missing\";\n\
         interface RealProps { label?: string }\n\
         export const Real = wrap<HTMLDivElement, RealProps>(\"div\");\n\
         export const Ghost = wrap<HTMLDivElement, MissingProps>(\"div\");\n",
    );
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    // Ghost's props live in a package that is not installed, as with Chakra's `@ark-ui` types in a source-only
    // checkout: it is still a component, and the unresolvable import says why its props are missing.
    assert_eq!(output.components.keys().collect::<Vec<_>>(), vec!["Ghost", "Real"]);
    assert_eq!(output.components["Real"].props.keys().collect::<Vec<_>>(), vec!["label"]);
    assert!(output.components["Ghost"].props.is_empty());
    assert!(
        output
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::UnresolvableImport && d.message.contains("MissingProps")),
        "diagnostics: {:?}",
        output.diagnostics
    );
}

// An interface may extend a type that is only reachable through a barrel file that re-exports it from a sibling.
#[test]
fn test_props_extended_through_a_reexporting_barrel_resolve() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("system")).unwrap();
    fs::create_dir_all(tmp.path().join("components")).unwrap();
    write_file(&tmp, "system/shared.ts", "export interface UnstyledProp { unstyled?: boolean }\n");
    write_file(&tmp, "system/index.ts", "export type { UnstyledProp } from \"./shared\";\n");
    write_file(
        &tmp,
        "components/only.tsx",
        "import type { UnstyledProp } from \"../system\";\n\
         export interface OnlyProps extends UnstyledProp { size?: number }\n\
         export function Only(props: OnlyProps) { return null; }\n",
    );
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    assert_eq!(output.components["Only"].props.keys().collect::<Vec<_>>(), vec!["size", "unstyled"]);
}

// Utility types in an extends clause or as the props type itself must honor a generic base and a `keyof` key set.
#[test]
fn test_utility_types_with_generic_bases_and_keyof_keys_resolve_exactly() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "a.tsx",
        "interface Gen<T> { x: T; y: string }\n\
         interface Base { a: number; b: number; c: number }\n\
         interface Other { b: number }\n\
         export interface P1 extends Pick<Gen<number>, \"x\"> { own: boolean }\n\
         export interface P2 extends Omit<Base, keyof Other> { own: boolean }\n\
         export function C1(props: P1) { return null }\n\
         export function C2(props: P2) { return null }\n\
         export function C4(props: Pick<Gen<number>, \"x\">) { return null }\n\
         export function C5(props: Omit<Base, keyof Other>) { return null }\n",
    );
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let props = |name: &str| -> Vec<&str> { output.components[name].props.keys().map(String::as_str).collect() };
    assert_eq!(props("C1"), vec!["own", "x"]);
    assert_eq!(props("C2"), vec!["a", "c", "own"]);
    assert_eq!(props("C4"), vec!["x"]);
    assert_eq!(props("C5"), vec!["a", "c"]);
}

// A generic interface's type arguments must reach its members, directly and through its own extends clause.
#[test]
fn test_generic_interface_type_arguments_are_substituted_into_props() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "a.tsx",
        "import type { FC } from \"react\";\n\
         interface Box<T> { value: T; other: string }\n\
         interface Wide<T> extends Box<T[]> { more: T }\n\
         export const A: FC<Box<string>> = () => null;\n\
         export function B(props: Box<number>) { return null }\n\
         export const C: FC<Box<\"a\" | \"b\">> = () => null;\n\
         export function D(props: Wide<boolean>) { return null }\n",
    );
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let kinds = |component: &str| -> Vec<(String, String)> {
        output.components[component]
            .props
            .iter()
            .map(|(name, prop)| (name.clone(), prop.prop_type.raw_string()))
            .collect()
    };
    let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
    assert_eq!(kinds("A"), vec![pair("other", "string"), pair("value", "string")]);
    assert_eq!(kinds("B"), vec![pair("other", "string"), pair("value", "number")]);
    assert_eq!(kinds("C"), vec![pair("other", "string"), pair("value", "\"a\" | \"b\"")]);
    assert_eq!(kinds("D"), vec![pair("more", "boolean"), pair("other", "string"), pair("value", "boolean[]")]);
}

// An alias whose expansion would print a conditional or mapped type's source keeps its name, as TypeScript prints it;
// literal unions and objects still expand.
#[test]
fn test_an_alias_over_an_unevaluated_type_keeps_its_name() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "a.tsx",
        "interface Override {}\n\
         type Size = \"sm\" | \"lg\";\n\
         type Breakpoint = Override extends { breakpoints: infer B } ? B : Size;\n\
         type Keys = { [K in Size]: string };\n\
         type StyleProp<V> = V | Partial<Record<Breakpoint, V>>;\n\
         type Spacing = string | Breakpoint;\n\
         type Slots = { root: Breakpoint; label: string };\n\
         export function C(props: {\n\
           size: Size;\n\
           from: Breakpoint;\n\
           keys: Keys;\n\
           m: StyleProp<Size>;\n\
           w: StyleProp<number | string>;\n\
           gap: Spacing;\n\
           slots: Slots;\n\
         }) { return null }\n",
    );
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let kinds: Vec<(String, String)> =
        output.components["C"].props.iter().map(|(name, prop)| (name.clone(), prop.prop_type.raw_string())).collect();
    let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
    assert_eq!(
        kinds,
        vec![
            pair("from", "Breakpoint"),
            pair("gap", "Spacing"),
            pair("keys", "Keys"),
            pair("m", "StyleProp<Size>"),
            pair("size", "\"sm\" | \"lg\""),
            pair("slots", "{ root: Breakpoint; label: string }"),
            pair("w", "StyleProp<number | string>"),
        ]
    );
}

// Barrels often import a name and then export it, instead of writing `export … from`; both must be followed.
#[test]
fn test_barrel_that_imports_then_exports_a_name_is_followed() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("system")).unwrap();
    fs::create_dir_all(tmp.path().join("components")).unwrap();
    write_file(
        &tmp,
        "system/shared.ts",
        "export interface UnstyledProp { unstyled?: boolean }\nexport interface Other { other?: string }\n",
    );
    write_file(
        &tmp,
        "system/index.ts",
        "import type { UnstyledProp, Other as Renamed } from \"./shared\";\nexport type { UnstyledProp, Renamed as Other };\n",
    );
    write_file(
        &tmp,
        "components/only.tsx",
        "import type { UnstyledProp, Other } from \"../system\";\n\
         export interface OnlyProps extends UnstyledProp, Other { size?: number }\n\
         export function Only(props: OnlyProps) { return null; }\n",
    );
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    assert_eq!(output.components["Only"].props.keys().collect::<Vec<_>>(), vec!["other", "size", "unstyled"]);
}

// Type arguments travel as structured types; each case here was silently wrong while they travelled as display text.
#[test]
fn test_structured_type_arguments_resolve_exactly() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "a.tsx",
        "import type { FC, ComponentPropsWithoutRef } from \"react\";\n\
         interface Base { T: string; number: boolean; other: number }\n\
         interface QuotedKey<T> extends Pick<Base, \"T\"> { own: T }\n\
         export function Quoted(props: QuotedKey<number>) { return null }\n\
         type Infinity = { x: string };\n\
         interface Box<T> { value: T }\n\
         export function Inf(props: Box<Infinity>) { return null }\n\
         export function Fn(props: Box<(a: string) => void>) { return null }\n\
         export function Arr(props: Box<(\"x\" | \"y\")[]>) { return null }\n\
         interface O1 { a?: string; b?: number }\n\
         export interface Req extends Required<O1> { own?: boolean }\n\
         export function R(props: Req) { return null }\n\
         interface ButtonProps { variant?: string; size?: number }\n\
         export function Button(props: ButtonProps) { return <button /> }\n\
         export interface Wrapped extends ComponentPropsWithoutRef<typeof Button> { own?: boolean }\n\
         export function W(props: Wrapped) { return null }\n\
         type Wrap<T> = { inner: T };\n\
         export const Nested: FC<Box<Wrap<{ b: string }>>> = () => null;\n",
    );
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let props = |component: &str| -> Vec<(String, String, bool)> {
        output.components[component]
            .props
            .iter()
            .map(|(name, p)| (name.clone(), p.prop_type.raw_string(), p.required))
            .collect()
    };
    let p = |n: &str, t: &str, r: bool| (n.to_owned(), t.to_owned(), r);
    // The key "T" is a string, not the type parameter T.
    assert_eq!(props("Quoted"), vec![p("T", "string", true), p("own", "number", true)]);
    // A type named Infinity is that type, not the number.
    assert_eq!(props("Inf"), vec![p("value", "{ x: string }", true)]);
    assert_eq!(props("Fn"), vec![p("value", "(a: string) => void", true)]);
    assert!(matches!(output.components["Arr"].props["value"].prop_type, PropType::Array(_)));
    // Required<O1> makes O1's props required.
    assert_eq!(props("R"), vec![p("a", "string", true), p("b", "number", true), p("own", "boolean", false)]);
    // ComponentPropsWithoutRef<typeof Button> is Button's props, not an element called "typeof button".
    assert_eq!(props("W"), vec![p("own", "boolean", false), p("size", "number", false), p("variant", "string", false)]);
    assert!(output.components["W"].inheritance.is_empty());
    // Distinct nested object arguments are distinct visits, not a cycle.
    assert!(
        !output.diagnostics.iter().any(|d| d.code == DiagnosticCode::CircularReference),
        "diagnostics: {:?}",
        output.diagnostics
    );
}

// Pick/Omit key sets resolve from any key expression: literals, unions, aliases, Exclude, keyof and parameters.
#[test]
fn test_key_sets_resolve_exactly() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "a.tsx",
        "interface Base { a: string; b: number; c: boolean }\n\
         interface Other { b: number }\n\
         type Keys = \"a\" | \"b\";\n\
         type More = \"c\";\n\
         export interface Picked extends Pick<Base, Keys> { own?: boolean }\n\
         export function P1(props: Picked) { return null }\n\
         export function P2(props: Omit<Base, Keys>) { return null }\n\
         export function P3(props: Omit<Base, \"a\" | More>) { return null }\n\
         export function P4(props: Omit<Base, Exclude<keyof Base, \"a\">>) { return null }\n\
         type MyOmit<T, K extends keyof T> = Omit<T, K>;\n\
         export function P5(props: MyOmit<Base, \"b\">) { return null }\n\
         interface Extra { d?: string }\n\
         export function P6(props: Omit<Base & Extra, \"a\">) { return null }\n\
         export function P8(props: Omit<Base, Unknowable<Base>>) { return null }\n",
    );
    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let names = |c: &str| -> Vec<&str> { output.components[c].props.keys().map(String::as_str).collect() };
    assert_eq!(names("P1"), vec!["a", "b", "own"]);
    assert_eq!(names("P2"), vec!["c"]);
    assert_eq!(names("P3"), vec!["b"]);
    assert_eq!(names("P4"), vec!["a"]);
    assert_eq!(names("P5"), vec!["a", "c"]);
    assert_eq!(names("P6"), vec!["b", "c", "d"]);
    // An unknowable key set omits nothing and says so.
    assert_eq!(names("P8"), vec!["a", "b", "c"]);
    assert!(
        output.diagnostics.iter().any(|d| d.message.starts_with("Cannot determine the keys of Omit<")),
        "diagnostics: {:?}",
        output.diagnostics
    );
}

// Excludes describe paths inside the project; the directory the project lives in must not trigger them.
#[test]
fn test_excludes_match_inside_the_source_directory_only() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("work.test.repo").join("node_modules-backup").join("src");
    fs::create_dir_all(root.join("__snapshots__")).unwrap();
    fs::write(
        root.join("Button.tsx"),
        "interface P { a?: string }\nexport function Button(props: P) { return <b /> }\n",
    )
    .unwrap();
    fs::write(
        root.join("Button.test.tsx"),
        "interface Q { a?: string }\nexport function InTest(props: Q) { return <b /> }\n",
    )
    .unwrap();
    fs::write(
        root.join("__snapshots__").join("Snap.tsx"),
        "interface S { a?: string }\nexport function Snap(props: S) { return <b /> }\n",
    )
    .unwrap();
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(root).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    assert_eq!(output.components.keys().collect::<Vec<_>>(), vec!["Button"]);
}

// ── test_overlapping_src_dirs_two_directories_deduplicates_without_a_duplicate_listing ─
//
// Corrected finding: the insert-return check above only catches a genuine
// 3+-way same-key collision. A realistic 2-directory overlap (e.g.
// `["./src", "./src/components"]` both walking the same physical file)
// produces exactly 2 duplicate mappings for the identical file that get
// keyed DIFFERENTLY — the first gets the plain "Button" key, the second
// gets the disambiguated "Button (<path>)" key — so they never collide at
// `insert` and both silently appear as separate entries in the output.
// `discover_files` must dedup identical canonical paths before they ever
// become two separate component_mappings.

#[test]
fn test_overlapping_src_dirs_two_directories_deduplicates_without_a_duplicate_listing() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = TempDir::new_in(manifest_dir).unwrap();
    let components_dir = tmp.path().join("components");
    fs::create_dir(&components_dir).unwrap();
    fs::write(components_dir.join("Button.tsx"), "export function Button(props: { a?: string }) { return null; }\n")
        .unwrap();

    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let nested = Utf8PathBuf::from_path_buf(components_dir).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root, nested],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let button_keys: Vec<&String> = output.components.keys().filter(|k| k.starts_with("Button")).collect();
    assert_eq!(
        button_keys.len(),
        1,
        "the same physical file discovered via two overlapping src_dirs must produce exactly one \
         component entry, got keys: {button_keys:?}"
    );
    assert_eq!(output.stats.files_parsed, 1, "the overlapping file must be parsed once, not twice");
}

// ── SPEC-PIPELINE-001 AC-001: built-in __snapshots__/node_modules
// substring exclusions.

#[test]
fn test_discover_files_excludes_snapshots_and_node_modules() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");
    fs::create_dir_all(tmp.path().join("__snapshots__")).unwrap();
    write_file(&tmp, "__snapshots__/Button.snap.tsx", "export const X = 1;");
    fs::create_dir_all(tmp.path().join("node_modules/some-lib")).unwrap();
    write_file(&tmp, "node_modules/some-lib/index.tsx", "export const Y = 1;");

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, _diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"));
    assert!(!files.iter().any(|f| f.as_str().contains("__snapshots__")), "should skip __snapshots__");
    assert!(!files.iter().any(|f| f.as_str().contains("node_modules")), "should skip node_modules by default");
}

// ── SPEC-PIPELINE-001 AC-001: .gitignore/.git/info/exclude/.ignore work-
// tree matrix. Previously zero coverage — the most intricate criterion in
// the whole spec-drift review (see its own revision_note: 7 gate rounds,
// verified line-by-line against the `ignore` crate's real source). The
// only custom logic discover.rs has here is one line —
// `git_ignore(!dir_is_in_node_modules)` — everything else is the `ignore`
// crate's own default behavior for `.git/info/exclude` and `.ignore`
// files, which these tests exercise via real `.git`/`.gitignore`/`.ignore`
// fixtures rather than trusting the crate's docs alone.

#[test]
fn gitignore_excludes_a_matching_file_inside_a_real_git_work_tree() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".git")).unwrap();
    write_file(&tmp, ".gitignore", "Ignored.tsx\n");
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");
    write_file(&tmp, "Ignored.tsx", "export const Skip = () => null;");

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, _diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"));
    assert!(!names.contains(&"Ignored.tsx"), ".gitignore should exclude the matching file in a real git work tree");
}

#[test]
fn gitignore_has_no_effect_when_the_src_dir_string_contains_node_modules() {
    let tmp = TempDir::new().unwrap();
    let lib_dir = tmp.path().join("node_modules").join("some-lib");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::create_dir_all(lib_dir.join(".git")).unwrap();
    fs::write(lib_dir.join(".gitignore"), "Ignored.tsx\n").unwrap();
    fs::write(lib_dir.join("Button.tsx"), "export const Button = () => null;").unwrap();
    fs::write(lib_dir.join("Ignored.tsx"), "export const Skip = () => null;").unwrap();

    let dir = Utf8PathBuf::from_path_buf(lib_dir).unwrap();
    let (files, _diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"));
    assert!(
        names.contains(&"Ignored.tsx"),
        ".gitignore must have NO effect when the src_dir's own configured path string contains \
         'node_modules', even with a real .git directory present, got {names:?}"
    );
}

#[test]
fn ignore_file_excludes_a_matching_file_with_no_exception() {
    // Unlike .gitignore, .ignore excludes unconditionally — test it inside
    // a node_modules-string src_dir specifically, where .gitignore above
    // was proven inert, to prove .ignore really is a separate, unaffected
    // mechanism (not just "also disabled by the same flag").
    let tmp = TempDir::new().unwrap();
    let lib_dir = tmp.path().join("node_modules").join("some-lib");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::write(lib_dir.join(".ignore"), "Ignored.tsx\n").unwrap();
    fs::write(lib_dir.join("Button.tsx"), "export const Button = () => null;").unwrap();
    fs::write(lib_dir.join("Ignored.tsx"), "export const Skip = () => null;").unwrap();

    let dir = Utf8PathBuf::from_path_buf(lib_dir).unwrap();
    let (files, _diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"));
    assert!(!names.contains(&"Ignored.tsx"), ".ignore should exclude matching files with no exception");
}

#[test]
fn gitignore_has_no_effect_outside_any_git_work_tree_but_ignore_still_does() {
    let tmp = TempDir::new().unwrap();
    // Deliberately NO .git directory anywhere in this tree.
    write_file(&tmp, ".gitignore", "GitignoredButIncluded.tsx\n");
    write_file(&tmp, ".ignore", "TrulyIgnored.tsx\n");
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");
    write_file(&tmp, "GitignoredButIncluded.tsx", "export const A = () => null;");
    write_file(&tmp, "TrulyIgnored.tsx", "export const B = () => null;");

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, _diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"));
    assert!(
        names.contains(&"GitignoredButIncluded.tsx"),
        ".gitignore has no effect outside a git work tree, got {names:?}"
    );
    assert!(!names.contains(&"TrulyIgnored.tsx"), ".ignore still excludes matching files outside a git work tree");
}

#[test]
fn a_src_dir_physically_under_node_modules_but_not_named_that_way_is_not_exempt() {
    // Negative pair: src_dir is physically located inside a node_modules
    // directory on disk, but reached via a symlink whose own literal
    // string does NOT contain "node_modules" — dir_is_in_node_modules is
    // a string check on the CONFIGURED src_dir, not the real filesystem
    // location, so ordinary .gitignore rules (ancestor-.git-gated) must
    // still apply here, unlike the exempted case above.
    let tmp = TempDir::new().unwrap();
    let real_dir = tmp.path().join("vendor").join("node_modules").join("mylib").join("src");
    fs::create_dir_all(&real_dir).unwrap();
    fs::create_dir_all(real_dir.join(".git")).unwrap();
    fs::write(real_dir.join(".gitignore"), "Ignored.tsx\n").unwrap();
    fs::write(real_dir.join("Button.tsx"), "export const Button = () => null;").unwrap();
    fs::write(real_dir.join("Ignored.tsx"), "export const Skip = () => null;").unwrap();

    let link = tmp.path().join("link-without-that-substring");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_dir, &link).unwrap();
    #[cfg(not(unix))]
    return; // No portable symlink API on this platform — skip rather than fail.

    let dir = Utf8PathBuf::from_path_buf(link).unwrap();
    assert!(
        !dir.as_str().contains("node_modules"),
        "test fixture invalid: the configured src_dir string must not contain 'node_modules'"
    );
    let (files, _diagnostics) = discover_files(&[dir], &[]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(names.contains(&"Button.tsx"));
    assert!(
        !names.contains(&"Ignored.tsx"),
        ".gitignore must still apply — the src_dir's configured string doesn't contain 'node_modules', \
         even though it's physically located inside a node_modules directory on disk, got {names:?}"
    );

    // Separately, and independent of gitignore: this fixture also proves the
    // pre-canonicalization substring-matching claim. Button.tsx's WALK-
    // EMITTED path (via the symlink, "link-without-that-substring") never
    // contains "node_modules", so the built-in node_modules-substring
    // exclusion never fires for it — but its FINAL, canonicalized file_path
    // (resolved after the exclusion check runs) does contain "node_modules",
    // since the symlink's real target is physically under one. If discover.rs
    // canonicalized before checking, this file would have been wrongly
    // excluded by the same built-in rule that skips real node_modules trees.
    let button = files.iter().find(|f| f.file_name() == Some("Button.tsx")).unwrap();
    assert!(
        button.as_str().contains("node_modules"),
        "test fixture invalid: the resolved file_path should resolve through the real on-disk \
         node_modules directory, got {button}"
    );
}

// ── SPEC-PIPELINE-001 AC-001: exclude_prefixes filters components after
// resolution, silently and by design.

#[test]
fn test_exclude_prefixes_filters_matching_components() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");
    write_file(&tmp, "InternalWidget.tsx", "export function InternalWidget(props: { x: string }) { return null; }\n");

    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        exclude_prefixes: vec!["Internal".to_string()],
        ..Default::default()
    };
    let output = extract(&options);

    assert!(output.components.contains_key("Button"));
    assert!(!output.components.contains_key("InternalWidget"), "excluded-prefix component must not appear");
    assert!(
        output.diagnostics.is_empty(),
        "exclude_prefixes is a deliberate, silent, opt-in filter — no diagnostic expected, got {:?}",
        output.diagnostics
    );
}

// ── SPEC-PIPELINE-001 AC-006: deterministic ordering across two separate
// extract() calls on the same unchanged source.

#[test]
fn test_extract_is_deterministic_across_repeated_calls() {
    let tmp = TempDir::new().unwrap();
    for name in ["Alpha", "Beta", "Gamma", "Delta"] {
        write_file(
            &tmp,
            &format!("{name}.tsx"),
            &format!("export function {name}(props: {{ x: string }}) {{ return null; }}\n"),
        );
    }

    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let first = extract(&options);
    let second = extract(&options);

    let first_keys: Vec<&String> = first.components.keys().collect();
    let second_keys: Vec<&String> = second.components.keys().collect();
    assert_eq!(first_keys, second_keys, "component key order must be identical across repeated extract() calls");
}

// ── SPEC-PIPELINE-001 AC-010: a triple same-file duplicate declaration
// produces exactly two entries (bare-name 1st occurrence, disambiguated
// 2nd/3rd collide, later wins deterministically) plus a collision
// diagnostic.

#[test]
fn test_triple_duplicate_declaration_produces_exactly_two_entries() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp,
        "Button.tsx",
        "export function Button(props: { a: string }) { return null; }\n\
         export function Button(props: { b: string }) { return null; }\n\
         export function Button(props: { c: string }) { return null; }\n",
    );

    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let button_keys: Vec<&String> = output.components.keys().filter(|k| k.starts_with("Button")).collect();
    assert_eq!(button_keys.len(), 2, "expected exactly 2 entries (bare + disambiguated), got {button_keys:?}");

    // Content, not presence: the bare-name entry (1st occurrence) must carry
    // props {a}, and the disambiguated entry must carry props from the LATER
    // (3rd) occurrence — {c} — not the 2nd's {b}, per AC-010's "iterated
    // later during Resolve/Collect" rule. Checking only the key count/names
    // would pass even if the 2nd occurrence's props won instead of the 3rd's.
    let bare = output.components.get("Button").expect("expected a bare-name entry");
    assert_eq!(
        bare.props.keys().collect::<Vec<_>>(),
        vec!["a"],
        "bare-name entry should keep the 1st occurrence's props, got {:?}",
        bare.props.keys().collect::<Vec<_>>()
    );

    let disambiguated_key = button_keys.iter().find(|k| k.as_str() != "Button").expect("expected a disambiguated key");
    let disambiguated = output.components.get(*disambiguated_key).unwrap();
    assert_eq!(
        disambiguated.props.keys().collect::<Vec<_>>(),
        vec!["c"],
        "disambiguated entry should carry the 3rd occurrence's props (the later of the two colliding \
         declarations), not the 2nd's, got {:?}",
        disambiguated.props.keys().collect::<Vec<_>>()
    );

    let collision_diag = output.diagnostics.iter().find(|d| d.code == DiagnosticCode::ComponentKeyCollision);
    assert!(collision_diag.is_some(), "expected a ComponentKeyCollision diagnostic, got {:?}", output.diagnostics);
    assert!(
        collision_diag.unwrap().message.contains("Button.tsx"),
        "collision diagnostic must name the colliding file path, got {:?}",
        collision_diag.unwrap().message
    );
}

// ── SPEC-PIPELINE-001 AC-022: exclude_patterns entries containing
// glob-invalid characters are inert (plain substring match).

#[test]
fn test_exclude_patterns_with_glob_invalid_characters_are_inert() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, diagnostics) = discover_files(&[dir], &["[unclosed*".to_string()]);

    let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
    assert!(
        names.contains(&"Button.tsx"),
        "a glob-invalid pattern that doesn't literally match must not exclude anything, got {names:?}"
    );
    assert!(diagnostics.is_empty(), "no error/panic expected for a glob-invalid but harmless pattern");
}

// ── SPEC-PIPELINE-001 AC-023: an empty-string exclude_patterns entry
// excludes every file (str::contains("") is always true).

#[test]
fn test_exclude_patterns_empty_string_excludes_everything() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Button.tsx", "export const Button = () => null;");
    write_file(&tmp, "Widget.tsx", "export const Widget = () => null;");

    let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let (files, diagnostics) = discover_files(&[dir], &["".to_string()]);

    assert!(files.is_empty(), "an empty-string pattern should exclude every file, got {files:?}");
    assert!(diagnostics.is_empty());
}

#[test]
fn a_diagnostic_shared_by_many_components_is_reported_once_listing_them() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "shared.ts", "import type { X } from './missing';\nexport interface Shared { x: X }\n");
    for name in ["Button", "Card"] {
        write_file(
            &tmp,
            &format!("{name}.tsx"),
            &format!(
                "import type {{ Shared }} from './shared';\nexport function {name}(props: Shared) {{ return null; }}\n"
            ),
        );
    }
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let summary: Vec<(DiagnosticCode, String, Vec<String>)> = output
        .diagnostics
        .iter()
        .map(|d| {
            (
                d.code.clone(),
                d.file.as_deref().unwrap_or("").rsplit('/').next().unwrap_or("").to_owned(),
                d.affects.clone(),
            )
        })
        .collect();
    let both = vec!["Button".to_string(), "Card".to_string()];
    assert_eq!(
        summary,
        // The missing module, then the type it would have provided: each once, listing both components.
        vec![
            (DiagnosticCode::UnresolvableImport, "shared.ts".to_string(), both.clone()),
            (DiagnosticCode::UnresolvableImport, "shared.ts".to_string(), both),
        ],
        "{:#?}",
        output.diagnostics
    );
}

#[test]
fn stats_count_every_dts_file_through_the_cache_and_its_hits() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "types.d.ts", "export interface Shared { size: number }\n");
    write_file(
        &tmp,
        "Button.tsx",
        "import type { Shared } from './types';\nexport function Button(props: Shared) { return null; }\n",
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let first = extract(&options).stats;
    let second = extract(&options).stats;

    assert_eq!((first.dts_files_parsed, first.dts_cache_hits), (1, 0));
    assert_eq!((second.dts_files_parsed, second.dts_cache_hits), (1, 1));
}

#[test]
fn a_generic_alias_in_prop_position_takes_the_call_sites_arguments() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "style.ts", "export type StyleProp<Value> = Value | Value[];\nexport type Maybe<T> = T | null;\n");
    write_file(
        &tmp,
        "Box.tsx",
        "import type { StyleProp, Maybe } from './style';\ntype Size = 'sm' | 'lg';\nexport function Box(props: { size: StyleProp<Size>; label: Maybe<string> }) { return null; }\n",
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };

    let output = extract(&options);

    let props = &output.components["Box"].props;
    // The alias's members join the union; inside the array it stays the alias's own enum.
    let size = PropType::LiteralUnion { members: vec!["sm".into(), "lg".into()], has_default: false };
    let (sm, lg) = (PropType::StringLiteral("sm".into()), PropType::StringLiteral("lg".into()));
    assert_eq!(props["size"].prop_type, PropType::Union(vec![sm, lg, PropType::Array(Box::new(size))]));
    assert_eq!(props["label"].prop_type, PropType::Union(vec![PropType::String, PropType::Null]));
}

fn extract_one(files: &[(&str, &str)]) -> ExtractionOutput {
    let tmp = TempDir::new().unwrap();
    for (name, body) in files {
        write_file(&tmp, name, body);
    }
    extract(&PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    })
}

/// The project is a directory inside the crate, so `react` and `typescript` resolve from the workspace's `node_modules`.
fn extract_with_installed_libs(files: &[(&str, &str)]) -> ExtractionOutput {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    for (name, body) in files {
        write_file(&tmp, name, body);
    }
    extract(&PipelineOptions { html_attributes: HtmlAttributeMode::Curated, ..full_mode_options_for(&tmp) })
}

fn severe(output: &ExtractionOutput) -> Vec<&crate::types::Diagnostic> {
    output.diagnostics.iter().filter(|d| d.severity != DiagnosticSeverity::Info).collect()
}

/// A type React declares that no builtin list names is React's, imported under its name or another.
#[test]
fn a_type_declared_by_react_is_not_unresolvable_even_when_no_builtin_list_names_it() {
    let output = extract_with_installed_libs(&[(
        "Box.tsx",
        "import type { UIEventHandler, Key, DOMAttributes as ReactDOMAttributes } from 'react';\n\
         export function Box(props: { onScroll?: UIEventHandler<HTMLDivElement>; id?: Key; dom?: ReactDOMAttributes<Element> }) { return null; }\n",
    )]);

    let props = &output.components["Box"].props;
    assert_eq!(
        props["onScroll"].prop_type,
        PropType::Named {
            name: "UIEventHandler".into(),
            args: vec![PropType::Named { name: "HTMLDivElement".into(), args: vec![] }]
        }
    );
    assert_eq!(props["id"].prop_type, PropType::Named { name: "Key".into(), args: vec![] });
    assert_eq!(severe(&output), Vec::<&crate::types::Diagnostic>::new());
}

/// Base UI's `{ [K in keyof ComponentPropsWithRef<"div">]: Wrap<ComponentPropsWithRef<"div">[K]> } & Own`: the
/// element's attributes, as `ComponentProps<"div">` lists them, beside the component's own props.
#[test]
fn a_mapped_type_over_an_elements_attributes_is_that_elements_layer() {
    let output = extract_with_installed_libs(&[(
        "Box.tsx",
        "import type * as React from 'react';\n\
         type Wrap<T> = T;\n\
         type Own = { own?: number };\n\
         export function Box(props: { [K in keyof React.ComponentPropsWithRef<\"div\">]: Wrap<React.ComponentPropsWithRef<\"div\">[K]> } & Own) { return null; }\n\
         export function Plain(props: React.ComponentPropsWithRef<\"div\"> & Own) { return null; }\n",
    )]);

    assert_eq!(severe(&output), Vec::<&crate::types::Diagnostic>::new());
    let layer = |name: &str| {
        let component = &output.components[name];
        (
            component.props.keys().cloned().collect::<Vec<_>>(),
            component.notable_inherited.keys().cloned().collect::<Vec<_>>(),
        )
    };
    assert_eq!(layer("Box"), layer("Plain"));
    assert_eq!(layer("Box").0, vec!["own".to_owned()]);
    assert!(layer("Box").1.contains(&"className".to_owned()));
}

/// Spectrum's illustrations: `props: IconProps & { size?: 'S' | 'M' }` and `let { size = 'M', ...rest } = props` in the
/// body. The inline field has the default as a declared one does.
#[test]
fn an_inline_object_field_takes_the_default_the_body_destructures() {
    let output = extract_one(&[(
        "Apps.tsx",
        "interface Base { tone?: string }\n\
         export default function Apps(props: Base & { size?: 'S' | 'M' }) {\n\
           let { size = 'M', ...rest } = props;\n  return null;\n}\n",
    )]);

    let size = &output.components["Apps"].props["size"];
    assert_eq!(size.default_value.as_ref().map(|d| d.value.as_str()), Some("'M'"));
    assert_eq!(output.components["Apps"].props["tone"].default_value, None);
}

/// Chakra's recipes: `{ [K in keyof Variant]?: … }` over an interface whose members carry `@default "md"`; the mapped
/// props keep the tag and so the default.
#[test]
fn a_mapped_type_keeps_the_default_tag_of_the_member_it_maps() {
    let output = extract_one(&[(
        "Box.tsx",
        "interface Variant {\n  /**\n   * The size.\n   * @default \"md\"\n   */\n  size: \"sm\" | \"md\" | \"lg\";\n  tone: \"a\" | \"b\";\n}\n\
         type VariantProps = { [K in keyof Variant]?: Variant[K] };\n\
         export function Box(props: VariantProps & { own?: number }) { return null; }\n",
    )]);

    let props = &output.components["Box"].props;
    assert_eq!(props["size"].default_value.as_ref().map(|d| d.value.as_str()), Some("\"md\""));
    assert_eq!(props["tone"].default_value, None);
}

#[test]
fn a_chakra_factory_config_supplies_inferred_recipe_props() {
    let output = extract_one(&[(
        "Group.tsx",
        r#"
declare function chakra(element: string, config: unknown): unknown;
const StyledGroup = chakra("div", {
  variants: {
    orientation: { horizontal: {}, vertical: {} },
    attached: { true: {} },
    grow: { true: {} },
    stacking: { "first-on-top": {}, "last-on-top": {} },
  },
});
type InferRecipeProps<T> = T extends unknown ? T : {};
type VariantProps = InferRecipeProps<typeof StyledGroup>;
interface GroupProps extends VariantProps { label?: string }
export function Group(props: GroupProps) { return <div>{props.label}</div>; }
"#,
    )]);
    assert_eq!(prop_names(&output, "Group"), ["attached", "grow", "label", "orientation", "stacking"]);
    assert_eq!(
        output.components["Group"].props["attached"].prop_type.raw_string(),
        "ConditionalValue<boolean | undefined>"
    );
    assert_eq!(
        output.components["Group"].props["orientation"].prop_type.raw_string(),
        "ConditionalValue<\"horizontal\" | \"vertical\" | undefined>"
    );
    assert!(
        output.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != crate::types::DiagnosticCode::OpaqueType || !diagnostic.message.contains("StyledGroup")
        }),
        "{:#?}",
        output.diagnostics
    );
}

/// A component whose props extend what `typeof recipe` infers (the config's `variants`), after `setup` declares `recipe`.
fn variant_bag_output(setup: &str) -> ExtractionOutput {
    let source = format!(
        "{setup}\n\
         type InferRecipeProps<T> = T extends unknown ? T : {{}};\n\
         type VariantProps = InferRecipeProps<typeof recipe>;\n\
         interface BoxProps extends VariantProps {{ label?: string }}\n\
         export function Box(props: BoxProps) {{ return <div>{{props.label}}</div>; }}\n"
    );
    extract_one(&[("Box.tsx", source.as_str())])
}

/// The variant bag is found by its shape, not by the name the call is imported under: `cva` as `c` still declares `size`
/// and `tone`, with the config at argument 1.
#[test]
fn a_variant_bag_is_found_under_an_aliased_cva_import() {
    let output = variant_bag_output(
        "import { cva as c } from \"class-variance-authority\";\n\
         const recipe = c(\"base\", { variants: { size: { sm: \"text-sm\", lg: [\"text-lg\"] }, tone: { a: `x`, b: \"y\" } } });",
    );
    assert_eq!(prop_names(&output, "Box"), ["label", "size", "tone"]);
    assert_eq!(
        output.components["Box"].props["size"].prop_type.raw_string(),
        "ConditionalValue<\"sm\" | \"lg\" | undefined>"
    );
    assert_eq!(
        output.components["Box"].props["tone"].prop_type.raw_string(),
        "ConditionalValue<\"a\" | \"b\" | undefined>"
    );
}

/// `tv({ variants })` keeps the config at argument 0, under an alias too.
#[test]
fn a_variant_bag_is_found_under_an_aliased_tv_import_with_the_config_first() {
    let output = variant_bag_output(
        "import { tv as t } from \"tailwind-variants\";\n\
         const recipe = t({ base: \"b\", variants: { size: { sm: \"s\", md: \"m\" } }, defaultVariants: { size: \"md\" } });",
    );
    assert_eq!(prop_names(&output, "Box"), ["label", "size"]);
    assert_eq!(
        output.components["Box"].props["size"].prop_type.raw_string(),
        "ConditionalValue<\"sm\" | \"md\" | undefined>"
    );
}

/// A library nobody listed, with the same config shape, in any argument position.
#[test]
fn a_variant_bag_is_found_for_a_library_the_extractor_has_never_heard_of() {
    let output = variant_bag_output(
        "declare function makeVariants(first: string, second: number, config: unknown): unknown;\n\
         const recipe = makeVariants(\"x\", 1, { variants: { density: { \"tight\": {}, \"loose\": { gap: 4 } } } });",
    );
    assert_eq!(prop_names(&output, "Box"), ["density", "label"]);
    assert_eq!(
        output.components["Box"].props["density"].prop_type.raw_string(),
        "ConditionalValue<\"tight\" | \"loose\" | undefined>"
    );
}

/// Real recipes mix literal groups with groups taken from another recipe or built by a call (Chakra's input addon:
/// `size: inputRecipe.variants!.size`, `variant: { … }`). The literal groups still declare their props.
#[test]
fn a_variant_bag_keeps_its_literal_groups_when_other_groups_are_references() {
    let output = variant_bag_output(
        "declare const inputRecipe: { variants: { size: Record<string, unknown> } };\n\
         declare function mapEntries(value: unknown, map: unknown): unknown;\n\
         declare function defineRecipe(config: unknown): unknown;\n\
         const recipe = defineRecipe({\n\
           variants: {\n\
             size: inputRecipe.variants!.size,\n\
             variant: { outline: { bg: \"a\" }, subtle: { bg: \"b\" } },\n\
             tone: mapEntries(inputRecipe, (key: string) => key),\n\
           },\n\
         });",
    );
    assert_eq!(prop_names(&output, "Box"), ["label", "variant"]);
    assert_eq!(
        output.components["Box"].props["variant"].prop_type.raw_string(),
        "ConditionalValue<\"outline\" | \"subtle\" | undefined>"
    );
}

/// An object with a `variants` property that is not a map of maps of styling is not a variant bag, and nothing is
/// invented from it: a plain string, a list, a group that is not an object, or a leaf that is a number.
#[test]
fn an_object_with_a_variants_property_that_is_not_a_variant_bag_declares_no_props() {
    for config in [
        "{ variants: \"size\" }",
        "{ variants: [\"a\", \"b\"] }",
        "{ variants: { size: \"sm\" } }",
        "{ variants: { size: { sm: 1, lg: 2 } } }",
        "{ variants: { size: { sm: true } } }",
        "{ variants: {} }",
    ] {
        let output = variant_bag_output(&format!(
            "declare function use(config: unknown): unknown;\nconst recipe = use({config});"
        ));
        assert_eq!(prop_names(&output, "Box"), ["label"], "{config}");
    }
}

/// A generic component's own type parameters are placeholders, the way an interface's are, and an ES2015 global
/// (`Iterable`) is TypeScript's.
#[test]
fn a_generic_components_type_parameter_and_an_es2015_global_are_not_unresolvable() {
    let output = extract_with_installed_libs(&[(
        "List.tsx",
        "interface ListProps<T> { items?: Iterable<T>; render?: (item: T) => null }\n\
         export function List<T extends object>(props: ListProps<T>) { return null; }\n\
         export const Wrapped = (function Inner<T extends object>(props: ListProps<T>) { return null; });\n",
    )]);

    assert_eq!(severe(&output), Vec::<&crate::types::Diagnostic>::new());
    assert_eq!(
        output.components["List"].props["items"].prop_type,
        PropType::Named { name: "Iterable".into(), args: vec![PropType::Named { name: "T".into(), args: vec![] }] }
    );
}

/// A class is also a type: `value: CalendarDate` names it, and nothing else about it is needed to print the name.
#[test]
fn a_class_used_as_a_type_is_declared() {
    let output = extract_one(&[
        ("date.ts", "export class CalendarDate { year = 0 }\n"),
        (
            "Cell.tsx",
            "import { CalendarDate } from './date';\n\
             export function Cell(props: { value: CalendarDate }) { return null; }\n",
        ),
    ]);

    assert_eq!(
        output.components["Cell"].props["value"].prop_type,
        PropType::Named { name: "CalendarDate".into(), args: vec![] }
    );
    assert_eq!(severe(&output), Vec::<&crate::types::Diagnostic>::new());
}

/// A namespace member names its siblings bare (csstype's `DataType.Color`), and the union keeps every member.
#[test]
fn a_namespace_sibling_in_an_alias_body_resolves_to_its_members() {
    let output = extract_one(&[(
        "Box.tsx",
        "namespace DataType { type Base = 'red' | 'blue'; export type Color = Base | 'green'; }\n\
         export function Box(props: { color: DataType.Color }) { return null; }\n",
    )]);

    let members = ["red", "blue", "green"].map(|m| PropType::StringLiteral(m.to_owned())).to_vec();
    assert_eq!(output.components["Box"].props["color"].prop_type, PropType::Union(members));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
}

/// `T extends unknown ? Omit<T, K> : never`, the idiom that distributes over a union, is the true branch.
#[test]
fn everything_extends_unknown_so_a_distributing_props_base_takes_its_true_branch() {
    let output = extract_one(&[(
        "Card.tsx",
        "type Distribute<T, K extends keyof any> = T extends unknown ? Omit<T, K> : never;\n\
         interface Base { id: string; hidden: boolean }\n\
         export function Card(props: Distribute<Base, 'hidden'> & { title: string }) { return null; }\n",
    )]);

    let mut card: Vec<&str> = output.components["Card"].props.keys().map(String::as_str).collect();
    card.sort_unstable();
    assert_eq!(card, vec!["id", "title"]);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
}

/// The nesting limit counts one type expression; aliases referring to aliases are declarations, bounded by the names in
/// progress, so a chain longer than the expression limit still resolves.
#[test]
fn a_chain_of_aliases_longer_than_the_expression_depth_limit_resolves() {
    let mut source = String::from("type A0 = 'x' | 'y';\n");
    for level in 1..40 {
        source.push_str(&format!("type A{level} = A{} | 'z{level}';\n", level - 1));
    }
    source.push_str("export function Box(props: { chain: A39 }) { return null; }\n");
    let output = extract_one(&[("Box.tsx", &source)]);

    let literals = |names: Vec<String>| PropType::Union(names.into_iter().map(PropType::StringLiteral).collect());
    let expected = ["x".to_owned(), "y".to_owned()].into_iter().chain((1..40).map(|level| format!("z{level}")));
    assert_eq!(output.components["Box"].props["chain"].prop_type, literals(expected.collect()));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
}

/// Mantine's `CssVars<V = CssVariable> = ... | CssVars<V>[]`: the argument of the recursive reference is the same type
/// at every level, so the cycle is found at the second one, as information, not by running out of depth.
#[test]
fn a_recursive_alias_with_a_defaulted_parameter_is_a_cycle_not_a_depth_overrun() {
    let output = extract_one(&[(
        "Box.tsx",
        "type Vars<V extends string = 'a'> = Record<V, string> | Vars<V>[];\n\
         export function Box(props: { vars?: Vars }) { return null; }\n",
    )]);

    let severe: Vec<_> = output.diagnostics.iter().filter(|d| d.severity != DiagnosticSeverity::Info).collect();
    assert!(severe.is_empty(), "{severe:?}");
    assert!(output.diagnostics.iter().any(|d| d.code == DiagnosticCode::CircularReference), "{:?}", output.diagnostics);
}

#[test]
fn a_project_declaration_named_like_a_builtin_is_that_declaration() {
    let output = extract_one(&[(
        "Card.tsx",
        "interface ReactNode { id: string }\ntype CSSProperties = 'compact' | 'wide';\n\
         interface Partial { size: number }\ninterface Required { name: string }\n\
         export function Card(props: { node: ReactNode; style: CSSProperties }) { return null; }\n\
         interface PanelProps extends Partial, Required { title: string }\n\
         export function Panel(props: PanelProps) { return null; }\n",
    )]);

    let card = &output.components["Card"].props;
    assert_eq!(card["node"].prop_type, PropType::Named { name: "ReactNode".into(), args: vec![] });
    assert_eq!(
        card["style"].prop_type,
        PropType::LiteralUnion { members: vec!["compact".into(), "wide".into()], has_default: false }
    );
    let mut panel: Vec<&str> = output.components["Panel"].props.keys().map(String::as_str).collect();
    panel.sort_unstable();
    assert_eq!(panel, vec!["name", "size", "title"]);
}

#[test]
fn react_builtins_keep_their_meaning_when_imported_from_react_or_unbound() {
    let output = extract_one(&[(
        "Card.tsx",
        "import type { ReactNode } from 'react';\n\
         export function Card(props: { a: ReactNode; b: React.ReactNode }) { return null; }\n",
    )]);

    let card = &output.components["Card"].props;
    assert_eq!((&card["a"].prop_type, &card["b"].prop_type), (&PropType::ReactNode, &PropType::ReactNode));
}

#[test]
fn a_builtin_name_whose_binding_does_not_resolve_falls_back_to_react_with_a_report() {
    let output = extract_one(&[
        ("types.ts", "export type { ReactNode } from 'react';\n"),
        ("Card.tsx", "import type { ReactNode } from './types';\nexport function Card(props: { a: ReactNode }) { return null; }\n"),
    ]);

    assert_eq!(output.components["Card"].props["a"].prop_type, PropType::ReactNode);
    let fallback: Vec<&str> = output
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("treated as React's 'ReactNode'"))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(fallback.len(), 1, "{:#?}", output.diagnostics);
    assert!(fallback[0].contains("is imported from './types'"), "{}", fallback[0]);
}

#[test]
fn parallel_json_writer_matches_serde_json_byte_for_byte() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "Size.ts", "export enum Size { Small = \"sm\", Large = \"lg\" }\n");
    write_file(
        &tmp,
        "Button.tsx",
        "import { Size } from './Size';\nimport { Missing } from './nowhere';\n\
         /** A button. */\nexport function Button(props: { size?: Size; label: string; extra: Missing }) {\n  \
         return <button>{props.label}</button>;\n}\n\
         export function Badge(props: { tone: 'info' | \"it's\" }) {\n  return <span />;\n}\n",
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);
    assert_eq!(output.components.keys().collect::<Vec<_>>(), ["Badge", "Button"]);
    assert!(!output.enums.is_empty() && !output.diagnostics.is_empty(), "every field must carry data: {output:#?}");

    assert_eq!(extraction_output_to_json(&output).unwrap(), serde_json::to_string(&output).unwrap());
}

/// A project with `src/ui/types.ts` and a Button importing it through the `@ui/*` alias, and `tsconfig` written as given.
fn aliased_project(tsconfig: &[(&str, &str)]) -> (TempDir, PipelineOptions) {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    std::fs::create_dir_all(root.join("src/ui")).unwrap();
    std::fs::write(root.join("src/ui/types.ts"), "export interface ButtonProps { label: string }\n").unwrap();
    std::fs::write(
        root.join("src/Button.tsx"),
        "import type { ButtonProps } from '@ui/types';\nexport function Button(props: ButtonProps) { return null; }\n",
    )
    .unwrap();
    for (name, body) in tsconfig {
        std::fs::write(root.join(name), body).unwrap();
    }
    let options = PipelineOptions {
        src_dirs: vec![root.join("src")],
        cache_dir: Some(root.join("cache")),
        tsconfig_path: Some(root.join(tsconfig[0].0)),
        ..Default::default()
    };
    (tmp, options)
}

#[test]
fn tsconfig_paths_resolve_through_block_comments_and_extends() {
    let commented = [(
        "tsconfig.json",
        "/* project config */\n{\n  // aliases\n  \"compilerOptions\": { /* root */ \"baseUrl\": \".\", \"paths\": { \"@ui/*\": [\"src/ui/*\"] } }\n}\n",
    )];
    let extended = [
        ("tsconfig.json", "{ \"extends\": \"./tsconfig.base.json\" }\n"),
        (
            "tsconfig.base.json",
            "{ \"compilerOptions\": { \"baseUrl\": \".\", \"paths\": { \"@ui/*\": [\"src/ui/*\"] } } }\n",
        ),
    ];
    for files in [&commented[..], &extended[..]] {
        let (_tmp, options) = aliased_project(files);

        let output = extract(&options);

        let props: Vec<&String> = output.components["Button"].props.keys().collect();
        assert_eq!(props, vec!["label"], "{}: {:#?}", files[0].1, output.diagnostics);
    }
}

#[test]
fn an_unusable_tsconfig_is_reported_once() {
    let (_tmp, options) = aliased_project(&[("tsconfig.json", "{ \"compilerOptions\": ")]);

    let output = extract(&options);

    let reports: Vec<&Diagnostic> =
        output.diagnostics.iter().filter(|d| d.message.starts_with("Could not use the tsconfig")).collect();
    assert_eq!(reports.len(), 1, "{:#?}", output.diagnostics);
    assert_eq!(reports[0].severity, DiagnosticSeverity::Warning);
}

/// A project root holding `files`, with `src` as the source directory and no tsconfig configured.
fn project_with(files: &[(&str, &str)]) -> (TempDir, Utf8PathBuf, PipelineOptions) {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    for (name, body) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    let options =
        PipelineOptions { src_dirs: vec![root.join("src")], cache_dir: Some(root.join("cache")), ..Default::default() };
    (tmp, root, options)
}

fn prop_names(output: &ExtractionOutput, component: &str) -> Vec<String> {
    output.components[component].props.keys().cloned().collect()
}

#[test]
fn the_nearest_tsconfig_applies_when_none_is_configured() {
    let (_tmp, _root, options) = project_with(&[
        ("tsconfig.json", "{ \"compilerOptions\": { \"baseUrl\": \".\", \"paths\": { \"@ui/*\": [\"src/ui/*\"] } } }"),
        ("src/ui/types.ts", "export interface ButtonProps { label: string }\n"),
        (
            "src/Button.tsx",
            "import type { ButtonProps } from '@ui/types';\nexport function Button(props: ButtonProps) { return null; }\n",
        ),
    ]);
    assert_eq!(options.tsconfig_path, None);

    let output = extract(&options);

    assert_eq!(prop_names(&output, "Button"), ["label"], "{:#?}", output.diagnostics);
}

#[test]
fn an_unusable_nearest_tsconfig_is_reported_once() {
    let (_tmp, root, options) = project_with(&[
        ("tsconfig.json", "{ \"compilerOptions\": "),
        ("src/Button.tsx", "export function Button(props: { label: string }) { return null; }\n"),
    ]);

    let output = extract(&options);

    let reports: Vec<&Diagnostic> =
        output.diagnostics.iter().filter(|d| d.message.starts_with("Could not use the tsconfig")).collect();
    assert_eq!(reports.len(), 1, "{:#?}", output.diagnostics);
    assert_eq!(reports[0].file.as_deref(), Some(root.join("tsconfig.json").as_str()));
}

#[test]
fn an_import_written_with_a_javascript_extension_finds_its_typescript_source() {
    let (_tmp, _root, options) = project_with(&[
        ("src/types.ts", "export interface ButtonProps { label: string }\n"),
        (
            "src/Button.tsx",
            "import type { ButtonProps } from './types.js';\nexport function Button(props: ButtonProps) { return null; }\n",
        ),
    ]);

    let output = extract(&options);

    assert_eq!(prop_names(&output, "Button"), ["label"], "{:#?}", output.diagnostics);
}

/// A workspace package whose `exports` reach its TypeScript source only through the `source` condition, linked into
/// `node_modules` the way a package manager links workspace packages.
#[cfg(unix)]
fn workspace_package_project(tsconfigs: &[(&str, &str)]) -> (TempDir, PipelineOptions) {
    let package =
        "{ \"name\": \"@acme/ui\", \"exports\": { \".\": { \"source\": \"./index.ts\", \"types\": \"./dist/index.d.ts\" } } }";
    let card = "import type { CardProps } from '@acme/ui';\nexport function Card(props: CardProps) { return null; }\n";
    let mut files = vec![
        ("src/ui/package.json", package),
        ("src/ui/index.ts", "export interface CardProps { title: string }\n"),
        ("src/Card.tsx", card),
    ];
    files.extend_from_slice(tsconfigs);
    let (tmp, root, options) = project_with(&files);
    std::fs::create_dir_all(root.join("node_modules/@acme")).unwrap();
    std::os::unix::fs::symlink(root.join("src/ui"), root.join("node_modules/@acme/ui")).unwrap();
    (tmp, options)
}

#[cfg(unix)]
#[test]
fn custom_conditions_from_an_extended_tsconfig_reach_a_workspace_packages_source() {
    let (_tmp, options) = workspace_package_project(&[
        ("tsconfig.base.json", "{ \"compilerOptions\": { \"customConditions\": [\"source\"], } }"),
        ("tsconfig.json", "// the project config\n{ \"extends\": \"./tsconfig.base\" }"),
    ]);

    let output = extract(&options);

    assert_eq!(prop_names(&output, "Card"), ["title"], "{:#?}", output.diagnostics);
}

#[cfg(unix)]
#[test]
fn without_custom_conditions_a_source_only_workspace_package_does_not_resolve() {
    let (_tmp, options) = workspace_package_project(&[("tsconfig.json", "{}")]);

    let output = extract(&options);

    assert_eq!(prop_names(&output, "Card"), Vec::<String>::new());
    let unresolved = output.diagnostics.iter().find(|d| d.code == DiagnosticCode::UnresolvableImport);
    assert!(unresolved.is_some_and(|d| d.message.contains("@acme/ui")), "{:#?}", output.diagnostics);
}

/// Mantine's shape, through two barrels: `factory<Payload>` returns `MantineComponent<Payload>`, a component type whose
/// props are the payload's props plus `component` and `renderRoot`. The wrapper-built component has those props, a name
/// both declare intersects as in TypeScript, and a callee with no known result leaves the payload's props.
#[test]
fn a_wrapper_built_component_gets_the_props_its_callees_result_type_adds() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    fs::create_dir_all(tmp.path().join("core/factory")).unwrap();
    write_file(&tmp, "core/factory/factory.tsx", CALLEE_FACTORY);
    write_file(
        &tmp,
        "core/factory/index.ts",
        "export { factory, opaque } from './factory';\nexport type { Factory } from './factory';\n",
    );
    write_file(&tmp, "core/index.ts", "export * from './factory';\n");
    write_file(
        &tmp,
        "Button.tsx",
        r#"
import { factory, opaque, Factory } from "./core";
export interface ButtonProps { label: string }
export interface LinkProps { label: string; component: "a" | "span" }
export type ButtonFactory = Factory<{ props: ButtonProps; ref: HTMLButtonElement }>;
export type LinkFactory = Factory<{ props: LinkProps; ref: HTMLAnchorElement }>;
export const Button = factory<ButtonFactory>((props) => <button>{props.label}</button>);
export const Link = factory<LinkFactory>((props) => <a>{props.label}</a>);
export const Plain = opaque<ButtonFactory>((props) => <button>{props.label}</button>);
export const Typed = factory<ButtonFactory>((props: ButtonProps) => <button>{props.label}</button>);
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let props = |component: &str| {
        output.components[component]
            .props
            .values()
            .map(|p| (p.name.clone(), p.prop_type.raw_string(), p.required, p.description.clone()))
            .collect::<Vec<_>>()
    };
    let prop = |name: &str, ty: &str, required: bool, description: &str| {
        (name.to_owned(), ty.to_owned(), required, description.to_owned())
    };
    let render_root = prop("renderRoot", "(props: Record<string, any>) => React.ReactNode", false, "");
    assert_eq!(
        props("Button"),
        [
            prop("component", "any", false, "Root element or component."),
            prop("label", "string", true, ""),
            render_root.clone()
        ]
    );
    // The payload's `component` and the result type's intersect, and `any` absorbs the intersection.
    assert_eq!(
        props("Link"),
        [prop("component", "any", true, "Root element or component."), prop("label", "string", true, ""), render_root,]
    );
    assert_eq!(props("Plain"), [prop("label", "string", true, "")]);
    // A typed render function makes this a HOC-wrapper detection; the callee still adds its props.
    assert_eq!(props("Typed"), props("Button"));
    let warnings: Vec<&str> = output
        .diagnostics
        .iter()
        .filter(|d| d.severity != DiagnosticSeverity::Info)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(warnings, Vec::<&str>::new());
}

#[test]
fn a_polymorphic_factory_result_through_local_aliases_adds_its_props() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "factory.tsx",
        r#"
import * as React from "react";
type PolymorphicComponentProps<C, Props> = C extends React.ElementType
  ? Props & { component?: C; renderRoot?: (props: Record<string, any>) => React.ReactNode }
  : Props & { component: React.ElementType; renderRoot?: (props: Record<string, any>) => React.ReactNode };
export interface Payload { props: Record<string, any>; defaultComponent: React.ElementType }
export type PolymorphicFactory<P extends Payload> = P;
export function polymorphicFactory<P extends Payload>(ui: (props: P['props']) => React.ReactNode) {
  type ComponentProps<C> = PolymorphicComponentProps<C, P['props']>;
  type Callable = <C = P['defaultComponent']>(props: ComponentProps<C>) => React.ReactElement;
  type ComponentProperties = Omit<React.FunctionComponent<ComponentProps<any>>, never>;
  type PolymorphicComponent = Callable & ComponentProperties;
  const Component = ui as unknown as PolymorphicComponent;
  return Component as PolymorphicComponent;
}

"#,
    );
    write_file(
        &tmp,
        "Button.tsx",
        r#"
import { polymorphicFactory, PolymorphicFactory } from './factory';
interface ButtonProps { label: string }
type ButtonPayload = PolymorphicFactory<{ props: ButtonProps; defaultComponent: 'button' }>;
export const Button = polymorphicFactory<ButtonPayload>((props) => <button>{props.label}</button>);
interface LinkProps { label: string; component: any }
type LinkPayload = PolymorphicFactory<{ props: LinkProps; defaultComponent: 'a' }>;
export const Link = polymorphicFactory<LinkPayload>((props) => <a>{props.label}</a>);
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);
    assert_eq!(prop_names(&output, "Button"), ["component", "label", "renderRoot"], "{:#?}", output.diagnostics);
    assert_eq!(prop_names(&output, "Link"), ["component", "label", "renderRoot"], "{:#?}", output.diagnostics);
    assert_eq!(output.components["Button"].props["label"].prop_type.raw_string(), "string");
    assert!(!output.components["Button"].props["component"].required);
    assert_eq!(output.components["Button"].props["component"].prop_type.raw_string(), "ElementType");
    assert!(!output.components["Button"].props["renderRoot"].required);
    assert_eq!(output.components["Link"].props["component"].prop_type.raw_string(), "any");
}

#[test]
fn a_local_alias_of_a_generic_component_factory_keeps_its_result() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "factory.tsx",
        r#"
import * as React from "react";
type ComponentProps<C, Props> = C extends React.ElementType
  ? Props & { component?: C; renderRoot?: (props: any) => any }
  : Props & { component: React.ElementType; renderRoot?: (props: any) => any };
export function createPolymorphicComponent<Default, Props>(component: any) {
  type Callable = <C = Default>(props: ComponentProps<C, Props>) => React.ReactElement;
  return component as Callable;
}

export const polymorphic = createPolymorphicComponent;
"#,
    );
    write_file(
        &tmp,
        "Box.tsx",
        r#"
import { polymorphic } from './factory';
interface BoxProps { label: string }
const _Box = (props: BoxProps) => <div>{props.label}</div>;
export const Box = polymorphic<'div', BoxProps>(_Box);
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);
    assert_eq!(prop_names(&output, "Box"), ["component", "label", "renderRoot"], "{:#?}", output.diagnostics);
    assert!(
        output.diagnostics.iter().all(|diagnostic| !diagnostic.message.contains("Cannot resolve type 'Props'")),
        "{:#?}",
        output.diagnostics
    );
}

#[test]
fn a_factory_returned_through_a_local_const_keeps_its_result_props() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "Table.tsx",
        r#"
import * as React from 'react';
interface FactoryPayload { props: Record<string, any> }
type Factory<P extends FactoryPayload> = P;
function factory<P extends FactoryPayload>(render: (props: P['props']) => React.ReactNode): React.FC<P['props'] & { component?: React.ElementType; renderRoot?: (props: any) => any }> {
  return render as any;
}
function tableElement<F extends FactoryPayload>(element: 'th') {
  const Component = factory<F>((props) => <th>{props.label}</th>);
  Component.displayName = 'TableTh';
  return Component;
}
interface TableThProps { label: string }
type TableThFactory = Factory<{ props: TableThProps }>;
export const TableTh = tableElement<TableThFactory>('th');
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);
    assert_eq!(prop_names(&output, "TableTh"), ["component", "label", "renderRoot"], "{:#?}", output.diagnostics);
}

/// An `export *` barrel reaches any exported value, not only types: `ComponentProps<typeof Button>` finds a component
/// declared as a const two barrels away, renamed exports included.
#[test]
fn an_export_star_barrel_reaches_an_exported_value() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    fs::create_dir_all(tmp.path().join("ui/button")).unwrap();
    write_file(
        &tmp,
        "ui/button/button.tsx",
        "export interface ButtonProps { label: string }\n\
         export const Button = (props: ButtonProps) => <button>{props.label}</button>;\n\
         const Ghost = (props: { faint?: boolean }) => <button />;\nexport { Ghost as GhostButton };\n",
    );
    write_file(&tmp, "ui/button/index.ts", "export * from './button';\n");
    write_file(&tmp, "ui/index.ts", "export * from './button';\n");
    write_file(
        &tmp,
        "App.tsx",
        r#"
import * as React from "react";
import { Button, GhostButton } from "./ui";
export function Fancy(props: React.ComponentProps<typeof Button> & { extra?: boolean }) { return null; }
export function Faint(props: React.ComponentProps<typeof GhostButton>) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let names = |component: &str| output.components[component].props.keys().cloned().collect::<Vec<_>>();
    assert_eq!(names("Fancy"), ["extra", "label"]);
    assert_eq!(names("Faint"), ["faint"]);
    let warnings: Vec<&str> = output
        .diagnostics
        .iter()
        .filter(|d| d.severity != DiagnosticSeverity::Info)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(warnings, Vec::<&str>::new());
}

/// A destructuring default is the component's default for that prop wherever the prop's type is declared: `variant`
/// and `size` come through `Pick<ComponentProps<typeof Button>, …>` (shadcn's `AlertDialogCancel`), not from an
/// interface the component names, and the component that declares them keeps having none.
#[test]
fn a_destructuring_default_applies_to_a_prop_another_component_declares() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "Button.tsx",
        "export interface ButtonProps { variant?: 'default' | 'outline'; size?: 'sm' | 'md'; label?: string }\n\
         export function Button(props: ButtonProps) { return <button>{props.label}</button>; }\n",
    );
    write_file(
        &tmp,
        "Cancel.tsx",
        r#"
import * as React from "react";
import { Button } from "./Button";
export function Cancel({ variant = "outline", size = 'md', ...rest }: Pick<React.ComponentProps<typeof Button>, "variant" | "size">) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let default_of = |component: &str, prop: &str| {
        output.components[component].props[prop].default_value.as_ref().map(|d| (d.value.clone(), d.computed))
    };
    assert_eq!(default_of("Cancel", "variant"), Some(("\"outline\"".to_owned(), false)));
    assert_eq!(default_of("Cancel", "size"), Some(("'md'".to_owned(), false)));
    assert_eq!(output.components["Cancel"].props["variant"].required, false);
    assert_eq!(default_of("Button", "variant"), None);
    assert_eq!(default_of("Button", "size"), None);
}

/// Two members of an intersection that both reach one type are not circular: `Optional<T, K>` is
/// `Pick<Partial<T>, K> & Omit<T, K>` (Ark UI's), and the second member used to be dropped as a cycle.
#[test]
fn intersection_members_that_share_a_base_type_each_keep_its_props() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "Field.tsx",
        r#"
export interface Base { id: string; value?: string; disabled?: boolean }
type Optional<T, K extends keyof T> = Pick<Partial<T>, K> & Omit<T, K>;
export function Field(props: Optional<Base, "id">) { return null; }
export function Both(props: Pick<Base, "id"> & Pick<Base, "disabled">) { return null; }
export function Rest(props: Pick<Base, "id"> & Omit<Base, "id">) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let names = |component: &str| output.components[component].props.keys().cloned().collect::<Vec<_>>();
    assert_eq!(names("Field"), ["disabled", "id", "value"]);
    assert_eq!(names("Both"), ["disabled", "id"]);
    assert_eq!(names("Rest"), ["disabled", "id", "value"]);
    // `id` is required in `Base`, and `Optional` makes it optional.
    assert_eq!(output.components["Field"].props["id"].required, false);
    assert_eq!(output.components["Rest"].props["id"].required, true);
    let circular: Vec<&str> = output
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::CircularReference)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(circular, Vec::<&str>::new());
}

/// Chakra's recipe props: a conditional on a literal picks its branch, indexed accesses read a generic interface's
/// member, and a mapped type over `keyof` an interface lists one prop per key. Every step is evaluated from known
/// operands, so nothing warns.
#[test]
fn recipe_props_evaluate_through_conditional_indexed_and_mapped_types() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp, "recipes.ts", RECIPES);
    write_file(
        &tmp,
        "Button.tsx",
        r#"
import type { RecipeProps } from "./recipes";
export interface ButtonProps extends RecipeProps<"button"> { loading?: boolean }
export function Button(props: ButtonProps) { return <button />; }
export interface OtherProps extends RecipeProps<"nope"> { label: string }
export function Other(props: OtherProps) { return <div />; }
export type FlagsProps = { [K in "compact" | "dense"]: boolean };
export function Flags(props: FlagsProps) { return <div />; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let props = |component: &str| {
        output.components[component]
            .props
            .values()
            .map(|p| (p.name.clone(), p.prop_type.raw_string(), p.required, p.description.clone()))
            .collect::<Vec<_>>()
    };
    let prop = |name: &str, ty: &str, required: bool, description: &str| {
        (name.to_owned(), ty.to_owned(), required, description.to_owned())
    };
    let recipe = prop("recipe", "string", false, "");
    assert_eq!(
        props("Button"),
        [
            prop("loading", "boolean", false, ""),
            recipe.clone(),
            prop("size", "ConditionalValue<\"sm\" | \"md\" | \"lg\" | undefined>", false, "The button's size."),
            prop("variant", "ConditionalValue<\"solid\" | \"outline\" | undefined>", false, ""),
        ]
    );
    assert_eq!(props("Other"), [prop("label", "string", true, ""), recipe]);
    assert_eq!(props("Flags"), [prop("compact", "boolean", true, ""), prop("dense", "boolean", true, "")]);
    let warnings: Vec<&str> = output
        .diagnostics
        .iter()
        .filter(|d| d.severity != DiagnosticSeverity::Info)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(warnings, Vec::<&str>::new());
}

/// Recipe types in Chakra's shape (`styled-system/generated/recipes.gen.ts`).
pub(crate) const RECIPES: &str = r#"
export interface SystemRecipeFn<VP, VM> { __type: Partial<VP>; variantMap: VM }
export interface ButtonVariant {
  /** The button's size. */
  size?: "sm" | "md" | "lg" | undefined;
  variant?: "solid" | "outline" | undefined;
}
export interface Conditions { _hover: string; md: string }
export type ConditionalValue<V> = V | Array<V | null> | { [K in keyof Conditions]?: ConditionalValue<V> };
export type ButtonVariantProps = { [K in keyof ButtonVariant]?: ConditionalValue<ButtonVariant[K]> | undefined };
export interface ConfigRecipes { button: SystemRecipeFn<ButtonVariantProps, unknown> }
export type RecipeProps<T> = T extends keyof ConfigRecipes
  ? ConfigRecipes[T]["__type"] & { recipe?: string | undefined }
  : { recipe?: string | undefined };
"#;

/// A factory module in Mantine's shape: `factory` asserts its result type, `opaque` declares none.
pub(crate) const CALLEE_FACTORY: &str = r#"
import * as React from "react";
export interface FactoryPayload { props: Record<string, any>; ref?: any }
export type Factory<Payload extends FactoryPayload> = Payload;
export type MantineComponent<Payload extends FactoryPayload> = React.NamedExoticComponent<
  Payload["props"] & React.RefAttributes<Payload["ref"]> & {
    /** Root element or component. */
    component?: any;
    renderRoot?: (props: Record<string, any>) => React.ReactNode;
  }
> & { extend: (input: any) => any };
export function factory<Payload extends FactoryPayload>(ui: (props: Payload["props"]) => React.ReactNode) {
  const Component = ui as any;
  return Component as MantineComponent<Payload>;
}
export function opaque<Payload extends FactoryPayload>(ui: (props: Payload["props"]) => React.ReactNode) {
  return ui;
}
"#;

#[test]
fn an_omitted_type_argument_takes_the_parameters_default() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "dom.ts",
        "export type FocusableElement = Element;\n\
         export interface DOMAttributes<T = FocusableElement> { onFocus?: (event: { currentTarget: T }) => void }\n",
    );
    write_file(
        &tmp,
        "Field.tsx",
        r#"
import type { DOMAttributes } from "./dom";
type Pair<A, B = A[]> = { first: A; second: B };
interface FieldProps extends DOMAttributes { label?: string }
interface InputProps extends DOMAttributes<HTMLInputElement> {}
interface Unbound<T> { value: T }
export function Field(props: FieldProps & { pair: Pair<string>; both: Pair<number, boolean>; unbound: Unbound<number> }) { return null; }
export function Input(props: InputProps) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let text = |component: &str, prop: &str| output.components[component].props[prop].prop_type.raw_string();
    assert_eq!(text("Field", "onFocus"), "(event: { currentTarget: FocusableElement }) => void");
    assert_eq!(text("Input", "onFocus"), "(event: { currentTarget: HTMLInputElement }) => void");
    assert_eq!(text("Field", "pair"), "{ first: string; second: string[] }");
    assert_eq!(text("Field", "both"), "{ first: number; second: boolean }");
    let warnings: Vec<&str> = output
        .diagnostics
        .iter()
        .filter(|d| d.severity != DiagnosticSeverity::Info)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(warnings, Vec::<&str>::new());
}

#[test]
fn literal_types_evaluate_as_typescript_does() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "Badge.tsx",
        r#"
type Size = "small" | "medium" | "large";
type Fallback<T> = { [P in keyof T]: T[P] | NonNullable<T[P]>[] };
interface Longhand { align?: "start" | "end" | undefined }
interface Styles extends Fallback<Longhand> {}
type Conditional<T> = T extends infer U ? U | U[] : never;
type WithVariant = { variant: "solid" | "ghost" };
type WithoutVariant = { props: string };
type VariantOf<P> = P["variant"] extends string ? P["variant"] | "custom" : string;
export function Badge(props: {
    size: Exclude<Size, "large"> | "default";
    picked: Extract<Size | number, string>;
    level: NonNullable<1 | 2 | null | undefined>;
    heading: `h${1 | 2 | 3}`;
    align: Conditional<Styles["align"]>;
    variant: VariantOf<WithVariant>;
    plain: VariantOf<WithoutVariant>;
}) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let text = |prop: &str| output.components["Badge"].props[prop].prop_type.raw_string();
    assert_eq!(text("size"), "\"small\" | \"medium\" | \"default\"");
    assert_eq!(text("picked"), "\"small\" | \"medium\" | \"large\"");
    assert_eq!(text("level"), "1 | 2");
    assert_eq!(text("heading"), "\"h1\" | \"h2\" | \"h3\"");
    assert_eq!(text("align"), "Conditional<\"start\" | \"end\" | undefined | (\"start\" | \"end\")[]>");
    assert_eq!(text("variant"), "\"solid\" | \"ghost\" | \"custom\"");
    assert_eq!(text("plain"), "string");
}

#[test]
fn a_conditional_alias_distributes_over_the_members_of_its_argument() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "sizes.ts",
        "export type Size = \"xs\" | \"sm\" | \"md\";\nexport const TONES = [\"a\", \"b\", \"c\"] as const;\n",
    );
    write_file(
        &tmp,
        "Tag.tsx",
        r#"
import type { Size, TONES } from "./sizes";
type Without<T, U> = T extends U ? never : T;
type OnlyStrings<T> = T extends string ? T : never;
type Tone = (typeof TONES)[number];
export function Tag(props: {
    size: Exclude<Size, "xs">;
    own: Without<Size | 4 | null, "sm" | null>;
    strings: OnlyStrings<Size | boolean | 7>;
    tone: Exclude<Tone, "b">;
    maybe: NonNullable<string | null | undefined>;
    flag: Exclude<boolean, true>;
    all: Exclude<Size, Size>;
}) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let text = |prop: &str| output.components["Tag"].props[prop].prop_type.raw_string();
    assert_eq!(text("size"), "\"sm\" | \"md\"");
    assert_eq!(text("own"), "\"xs\" | \"md\" | 4");
    assert_eq!(text("strings"), "\"xs\" | \"sm\" | \"md\"");
    assert_eq!(text("tone"), "\"a\" | \"c\"");
    assert_eq!(text("maybe"), "string");
    assert_eq!(text("flag"), "false");
    assert_eq!(text("all"), "never");
}

#[test]
fn a_generic_alias_with_a_long_expansion_is_shown_by_name_with_its_arguments() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let keys: Vec<String> = (0..80).map(|i| format!("\"k{i}\"")).collect();
    write_file(
        &tmp,
        "Grid.tsx",
        &format!(
            "type Key = {keys};\ntype Small = \"a\" | \"b\";\ntype Responsive<T> = T | T[] | {{ [bp: string]: T }};\n\
             export function Grid(props: {{ big: Responsive<Key>; small: Responsive<Small> }}) {{ return null; }}\n",
            keys = keys.join(" | ")
        ),
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let text = |prop: &str| output.components["Grid"].props[prop].prop_type.raw_string();
    assert_eq!(text("big"), "Responsive<Key>");
    assert!(text("small").starts_with("\"a\" | \"b\" | "), "got {}", text("small"));
}

#[test]
fn an_indexed_member_reads_through_imported_generic_ancestors_with_their_arguments_bound() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(&tmp, "base.ts", "export interface Base<T> { value?: T; tone?: \"a\" | \"b\" }\n");
    write_file(
        &tmp,
        "Field.tsx",
        r#"
import type { Base } from "./base";
interface Mid<U> extends Base<U> {}
interface Leaf extends Mid<number> {}
export function Field(props: { value: Leaf["value"]; tone: Leaf["tone"] }) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let props = &output.components["Field"].props;
    assert_eq!(props["value"].prop_type, PropType::Number);
    assert_eq!(
        props["tone"].prop_type,
        PropType::Union(vec![PropType::StringLiteral("a".into()), PropType::StringLiteral("b".into())])
    );
}

#[test]
fn a_union_lists_each_member_once_however_it_was_spelled() {
    let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    write_file(
        &tmp,
        "List.tsx",
        r#"
import * as React from "react";
type SectionElement = React.ReactElement | null;
type ItemElement = React.ReactElement | null;
type CollectionElement = SectionElement | ItemElement;
export function List(props: {
    children: CollectionElement | CollectionElement[];
    each: string[] | readonly string[];
    tone: "a" | ("a" | "b") | "b";
    onPick: (names: string[] | readonly string[]) => void;
}) { return null; }
"#,
    );
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = extract(&options);

    let props = &output.components["List"].props;
    let element = PropType::Union(vec![PropType::ReactNode, PropType::Null]);
    assert_eq!(
        props["children"].prop_type,
        PropType::Union(vec![PropType::ReactNode, PropType::Null, PropType::Array(Box::new(element))])
    );
    assert_eq!(props["each"].prop_type, PropType::Array(Box::new(PropType::String)));
    assert_eq!(
        props["tone"].prop_type,
        PropType::Union(vec![PropType::StringLiteral("a".into()), PropType::StringLiteral("b".into())])
    );
    assert_eq!(
        props["onPick"].prop_type,
        PropType::Function {
            type_params: vec![],
            params: vec![FunctionParam::new(Some("names".into()), PropType::Array(Box::new(PropType::String)))],
            return_type: Box::new(PropType::Void),
        }
    );
}

#[test]
fn an_exported_component_cast_uses_the_asserted_props() {
    let output = extract_one(&[(
        "Card.tsx",
        r#"
import { forwardRef, ForwardRefExoticComponent } from 'react';
type PropsWithoutRef<P> = P;
interface CardProps { label: string }
interface ItemProps<P> { title?: string; textValue?: string; href?: string }
let Card = forwardRef<HTMLDivElement, CardProps>((props, ref) => <div ref={ref}>{props.label}</div>);
let _Card = Card as ForwardRefExoticComponent<ItemProps<CardProps> & PropsWithoutRef<CardProps>>;
export { _Card as Card };
"#,
    )]);
    assert_eq!(
        prop_names(&output, "Card"),
        ["href", "label", "textValue", "title"],
        "components: {:#?}; diagnostics: {:#?}",
        output.components.keys().collect::<Vec<_>>(),
        output.diagnostics
    );
    assert_eq!(output.components["Card"].props["label"].prop_type.raw_string(), "string");
    assert_eq!(output.components["Card"].props["title"].prop_type.raw_string(), "string");
}

#[test]
fn react_props_without_ref_keeps_the_base_props_in_alias_and_extends_positions() {
    let output = extract_one(&[(
        "Cards.tsx",
        r#"
import type { PropsWithoutRef } from 'react';
interface Base { label: string; ref?: string }
type Alias = PropsWithoutRef<Base>;
interface Extended extends PropsWithoutRef<Base> {}
export function AliasCard(props: Alias) { return <div>{props.label}</div>; }
export function ExtendedCard(props: Extended) { return <div>{props.label}</div>; }
"#,
    )]);
    assert_eq!(prop_names(&output, "AliasCard"), ["label"]);
    assert_eq!(prop_names(&output, "ExtendedCard"), ["label"]);
}

// ── Optional and rest parameters survive into the output type ─────────────

fn function_params(output: &ExtractionOutput, component: &str, prop: &str) -> Vec<FunctionParam> {
    match &output.components[component].props[prop].prop_type {
        PropType::Function { params, .. } => params.clone(),
        other => panic!("expected a function type for {component}.{prop}, got {other:?}"),
    }
}

#[test]
fn a_function_prop_keeps_its_optional_and_rest_parameters() {
    let output = extract_one(&[(
        "Menu.tsx",
        r#"
export function Menu(props: {
    onChange?: (value: string, extra?: number, ...rest: boolean[]) => void;
    onClose: (...args: any[]) => void;
}) { return null; }
"#,
    )]);

    let flags = |params: &[FunctionParam]| {
        params.iter().map(|p| (p.name.clone().unwrap_or_default(), p.optional, p.rest)).collect::<Vec<_>>()
    };
    assert_eq!(
        flags(&function_params(&output, "Menu", "onChange")),
        [("value".to_owned(), false, false), ("extra".to_owned(), true, false), ("rest".to_owned(), false, true)]
    );
    assert_eq!(flags(&function_params(&output, "Menu", "onClose")), [("args".to_owned(), false, true)]);
    assert_eq!(
        output.components["Menu"].props["onChange"].prop_type.raw_string(),
        "(value: string, extra?: number, ...rest: boolean[]) => void"
    );
    assert_eq!(output.components["Menu"].props["onClose"].prop_type.raw_string(), "(...args: any[]) => void");
}

#[test]
fn a_method_signature_keeps_its_optional_and_rest_parameters() {
    let output = extract_one(&[(
        "Tabs.tsx",
        r#"
interface TabsProps {
    onSelect(key: string, more?: number, ...tail: string[]): void;
}
export function Tabs(props: TabsProps) { return null; }
"#,
    )]);

    assert_eq!(
        output.components["Tabs"].props["onSelect"].prop_type.raw_string(),
        "(key: string, more?: number, ...tail: string[]) => void"
    );
}

#[test]
fn a_destructured_parameter_stays_unnamed_next_to_an_optional_one() {
    let output = extract_one(&[(
        "Row.tsx",
        r#"
export function Row(props: { render: ({ id }: { id: string }, index?: number) => void }) { return null; }
"#,
    )]);

    let params = function_params(&output, "Row", "render");
    assert_eq!((params[0].name.clone(), params[0].optional, params[0].rest), (None, false, false));
    assert_eq!((params[1].name.clone(), params[1].optional, params[1].rest), (Some("index".to_owned()), true, false));
    assert_eq!(
        output.components["Row"].props["render"].prop_type.raw_string(),
        "(__0: { id: string }, index?: number) => void"
    );
}
