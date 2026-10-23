#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! Process-level tests for `main()`'s dispatch: each subcommand and the `-v` flag through the real binary.

use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

mod common;
use common::strip_ansi;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_caliper-docgen")).args(args).output().expect("failed to spawn the real binary")
}

/// A source directory holding one clean `Widget` component.
fn widget_dir() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("Widget.tsx"),
        "export function Widget(props: { label: string }) { return null; }\n",
    )
    .unwrap();
    dir
}

fn stdout_of(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn check_on_a_clean_tree_prints_no_diagnostics_and_exits_0() {
    let dir = widget_dir();
    let output = run(&["check", "--json", "--src", dir.path().to_str().unwrap()]);
    assert_eq!(stdout_of(&output), "[]\n");
}

#[test]
fn inspect_prints_the_component_and_its_props_table() {
    let dir = widget_dir();
    let output = run(&["inspect", "Widget", "--src", dir.path().to_str().unwrap()]);

    let stdout = strip_ansi(&stdout_of(&output));
    assert!(stdout.contains("Props (1)"), "{stdout}");
    let label_row = stdout.lines().find(|line| line.contains("label")).expect("a table row for `label`");
    assert!(label_row.contains("string") && label_row.contains('✓'), "{label_row}");
}

#[test]
fn completions_prints_a_shell_script() {
    let stdout = stdout_of(&run(&["completions", "bash"]));
    assert!(stdout.starts_with("_caliper__docgen() {"), "{}", stdout.lines().next().unwrap_or_default());
}

#[test]
fn verbose_flags_at_every_level_leave_stdout_as_the_json_payload() {
    let dir = widget_dir();
    for verbosity in ["-v", "-vv", "-vvv"] {
        let output = run(&[verbosity, "extract", "--json", "--src", dir.path().to_str().unwrap()]);

        let parsed: serde_json::Value = serde_json::from_str(&stdout_of(&output)).unwrap();
        let components: Vec<&String> = parsed["components"].as_object().unwrap().keys().collect();
        assert_eq!(components, ["Widget"], "{verbosity}");
    }
}

#[test]
fn check_prints_the_summary_to_stderr_unless_quiet() {
    let dir = widget_dir();
    let src = dir.path().to_str().unwrap();

    let noisy = run(&["check", "--src", src]);
    assert_eq!(noisy.status.code(), Some(0));
    let stderr = strip_ansi(&String::from_utf8(noisy.stderr).unwrap());
    assert!(stderr.contains("1 components  ·  0 enums  ·  0 warnings  ·  0 errors"), "{stderr}");

    let quiet = run(&["--quiet", "check", "--src", src]);
    assert_eq!(quiet.status.code(), Some(0));
    assert!(quiet.stderr.is_empty(), "{}", String::from_utf8_lossy(&quiet.stderr));
}

fn extract_to_stdout(format: &str) -> String {
    let dir = widget_dir();
    let output = run(&["--quiet", "extract", "--format", format, "--src", dir.path().to_str().unwrap()]);
    stdout_of(&output)
}

#[test]
fn extract_rdt_format_emits_react_docgen_typescript_props() {
    let parsed: serde_json::Value = serde_json::from_str(&extract_to_stdout("rdt")).unwrap();

    let label = &parsed["Widget"]["props"]["label"];
    assert_eq!(label["type"], serde_json::json!({ "name": "string" }));
    assert_eq!(label["required"], true);
}

#[test]
fn extract_storybook_format_emits_docgen_info_without_parents() {
    let parsed: serde_json::Value = serde_json::from_str(&extract_to_stdout("storybook")).unwrap();

    let label = &parsed["Widget"]["props"]["label"];
    assert_eq!(label["type"], serde_json::json!({ "name": "string" }));
    assert!(label.get("parent").is_none(), "storybook output carries no parent: {label}");
}

#[test]
fn extract_toon_format_lists_each_component_with_its_props() {
    let toon = extract_to_stdout("toon");

    assert!(toon.starts_with("docgenComponents[1]:\n  component:Widget:\n"), "{toon}");
    assert!(toon.contains("      label,true,string,,\n"), "{toon}");
}

/// Kills the child on drop so a failing assertion never leaks a watcher.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_bare_out_file_name_is_written_into_the_current_directory() {
    let src = widget_dir();
    let cwd = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_caliper-docgen"))
        .current_dir(cwd.path())
        .args(["--quiet", "extract", "--out", "out.json", "--src", src.path().to_str().unwrap()])
        .output()
        .expect("failed to spawn the real binary");

    assert_eq!(output.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(cwd.path().join("out.json")).unwrap()).unwrap();
    let components: Vec<&String> = written["components"].as_object().unwrap().keys().collect();
    assert_eq!(components, ["Widget"]);
    assert!(!cwd.path().join(".out.json.tmp").exists(), "the temp file must be renamed away");
}

/// A running `watch` over `src` with its stdout and stderr captured to files, plus the `--out` path it writes.
struct Watch {
    _child: KillOnDrop,
    out: std::path::PathBuf,
    stdout: std::path::PathBuf,
    stderr: std::path::PathBuf,
    _scratch: TempDir,
}

fn spawn_watch(src: &TempDir, quiet: bool) -> Watch {
    let scratch = TempDir::new().unwrap();
    let (out, stdout, stderr) =
        (scratch.path().join("out.json"), scratch.path().join("stdout.log"), scratch.path().join("stderr.log"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_caliper-docgen"));
    if quiet {
        command.arg("--quiet");
    }
    let child = command
        .args(["watch", "--src", src.path().to_str().unwrap(), "--out", out.to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&stdout).unwrap())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .spawn()
        .expect("failed to spawn the real binary");
    Watch { _child: KillOnDrop(child), out, stdout, stderr, _scratch: scratch }
}

impl Watch {
    /// Edits `Widget.tsx` until `--out` shows the edit, then returns what was written. An edit made before the watcher
    /// is listening is lost, so the edit is repeated.
    fn edit_until_out_is_written(&self, src: &TempDir) -> serde_json::Value {
        let widget = src.path().join("Widget.tsx");
        let deadline = Instant::now() + Duration::from_secs(30);
        // `--out` is written at startup, from the files as they were before the edit.
        while !self.out.exists() {
            assert!(Instant::now() < deadline, "watch never wrote --out at startup");
            std::thread::sleep(Duration::from_millis(50));
        }
        loop {
            std::fs::write(
                &widget,
                "export function Widget(props: { label: string; size: number }) { return null; }\n",
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(300));
            if let Ok(json) = std::fs::read_to_string(&self.out) {
                let written: serde_json::Value = serde_json::from_str(&json).unwrap();
                if written["components"]["Widget"]["props"]["size"].is_string() {
                    return written;
                }
            }
            assert!(Instant::now() < deadline, "watch never wrote the edit to --out");
        }
    }

    fn logged(&self, stream: &std::path::Path) -> String {
        strip_ansi(&std::fs::read_to_string(stream).unwrap())
    }
}

/// The temp file is renamed onto the target, so it must live beside it: a read-only cwd can only succeed if nothing
/// is written there.
#[cfg(unix)]
#[test]
fn out_is_staged_next_to_the_target_not_in_the_current_directory() {
    use std::os::unix::fs::PermissionsExt;

    let (src, cwd, target_dir) = (widget_dir(), TempDir::new().unwrap(), TempDir::new().unwrap());
    let out = target_dir.path().join("out.json");
    std::fs::set_permissions(cwd.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_caliper-docgen"))
        .current_dir(cwd.path())
        .args(["--quiet", "extract", "--src", src.path().to_str().unwrap()])
        .args(["--cache-dir", target_dir.path().join("cache").to_str().unwrap(), "--out", out.to_str().unwrap()])
        .output()
        .expect("failed to spawn the real binary");
    // Restore write access so the temp dir can be cleaned up.
    std::fs::set_permissions(cwd.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(output.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert!(written["components"]["Widget"].is_object());
}

#[test]
fn watch_reports_a_change_and_writes_out_what_extract_produces() {
    let src = widget_dir();
    let watch = spawn_watch(&src, false);

    let watched = watch.edit_until_out_is_written(&src);

    let extracted: serde_json::Value =
        serde_json::from_str(&stdout_of(&run(&["extract", "--json", "--src", src.path().to_str().unwrap()]))).unwrap();
    let props: Vec<&String> = watched["components"]["Widget"]["props"].as_object().unwrap().keys().collect();
    assert_eq!(props, ["label", "size"]);
    assert_eq!(watched["components"], extracted["components"], "watch and extract must agree on the same tree");
    assert_eq!(watched["props"], extracted["props"], "watch and extract must agree on the props the ids name");

    let stdout = watch.logged(&watch.stdout);
    assert!(
        stdout.contains(&format!("watching {}", src.path().display())),
        "the banner names the watched dir: {stdout}"
    );
    assert!(stdout.contains("Widget.tsx  Widget"), "the changed file and component are reported: {stdout}");
    let stderr = watch.logged(&watch.stderr);
    assert!(
        stderr.contains("1 components  ·  0 enums  ·  0 warnings  ·  0 errors"),
        "the first extraction is summarized: {stderr}"
    );
}

#[test]
fn watch_writes_out_before_any_change() {
    let src = widget_dir();
    let watch = spawn_watch(&src, true);

    let deadline = Instant::now() + Duration::from_secs(30);
    let written = loop {
        if let Ok(json) = std::fs::read_to_string(&watch.out) {
            break serde_json::from_str::<serde_json::Value>(&json).unwrap();
        }
        assert!(Instant::now() < deadline, "watch never wrote --out");
        std::thread::sleep(Duration::from_millis(50));
    };

    let extracted: serde_json::Value =
        serde_json::from_str(&stdout_of(&run(&["extract", "--json", "--src", src.path().to_str().unwrap()]))).unwrap();
    assert_eq!(written["components"], extracted["components"]);
}

#[test]
fn a_quiet_watch_still_writes_out_but_prints_nothing() {
    let src = widget_dir();
    let watch = spawn_watch(&src, true);

    watch.edit_until_out_is_written(&src);

    assert_eq!(watch.logged(&watch.stdout), "");
    assert_eq!(watch.logged(&watch.stderr), "");
}

/// A tree whose component names a type that doesn't exist, so extraction reports a warning.
fn dir_with_an_unresolved_type() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("Widget.tsx"), "export function Widget(props: Missing) { return null; }\n").unwrap();
    dir
}

#[test]
fn check_exit_codes_reach_the_shell() {
    let clean = widget_dir();
    let warned = dir_with_an_unresolved_type();
    let code = |args: &[&str]| run(args).status.code();

    assert_eq!(code(&["--quiet", "check", "--src", clean.path().to_str().unwrap()]), Some(0));
    assert_eq!(code(&["--quiet", "check", "--src", warned.path().to_str().unwrap()]), Some(0));
    assert_eq!(code(&["--quiet", "check", "--strict", "--src", warned.path().to_str().unwrap()]), Some(1));
    assert_eq!(code(&["--quiet", "check", "--src", "/nonexistent/docgen-dir"]), Some(2));
}

#[test]
fn extract_reports_diagnostics_on_stderr_only_when_neither_quiet_nor_json() {
    let dir = dir_with_an_unresolved_type();
    let src = dir.path().to_str().unwrap();

    let output = run(&["extract", "--src", src]);
    let payload: serde_json::Value = serde_json::from_str(&stdout_of(&output)).unwrap();
    assert!(payload["components"]["Widget"].is_object());
    let stderr = strip_ansi(&String::from_utf8(output.stderr).unwrap());
    assert!(stderr.contains("1 components  ·  0 enums  ·  1 warnings  ·  0 errors"), "{stderr}");
    assert!(stderr.contains("[warn] ") && stderr.contains("Cannot resolve type 'Missing'"), "{stderr}");

    assert!(run(&["extract", "--json", "--src", src]).stderr.is_empty(), "--json prints no human report");
    assert!(run(&["--quiet", "extract", "--src", src]).stderr.is_empty(), "--quiet prints no human report");
}

#[test]
fn extract_finds_docgen_config_ts_in_the_working_directory_and_applies_it() {
    // tsx, which evaluates the config, is only installed under apps/validate.
    let project = TempDir::new_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/validate")).unwrap();
    for (dir, component) in [("components", "Widget"), ("elsewhere", "Gadget")] {
        std::fs::create_dir(project.path().join(dir)).unwrap();
        std::fs::write(
            project.path().join(dir).join(format!("{component}.tsx")),
            format!("export function {component}(props: {{ label: string }}) {{ return null; }}\n"),
        )
        .unwrap();
    }
    std::fs::write(project.path().join("docgen.config.ts"), "export default { srcDirs: ['components'] };\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_caliper-docgen"))
        .current_dir(project.path())
        .args(["--quiet", "extract", "--json"])
        .output()
        .expect("failed to spawn the real binary");

    let parsed: serde_json::Value = serde_json::from_str(&stdout_of(&output)).unwrap();
    let components: Vec<&String> = parsed["components"].as_object().unwrap().keys().collect();
    assert_eq!(components, ["Widget"], "only the configured srcDirs are scanned");
}
