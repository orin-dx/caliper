use super::*;

#[test]
fn config_schema_maps_recognized_fields_onto_pipeline_options() {
    let json = r#"{
        "srcDirs": ["app/components"],
        "htmlAttributes": "full",
        "extraBuiltins": ["MyCustomType", "AnotherType"]
    }"#;
    let schema: DocgenConfigSchema = serde_json::from_str(json).expect("valid config JSON");
    let opts = schema.into_pipeline_options().expect("valid config");

    assert_eq!(opts.src_dirs, vec![camino::Utf8PathBuf::from("app/components")]);
    assert_eq!(opts.html_attributes, HtmlAttributeMode::Full);
    let extra_builtins: std::collections::BTreeSet<String> =
        opts.extra_builtins.iter().map(|s| s.to_string()).collect();
    assert_eq!(extra_builtins, ["AnotherType", "MyCustomType"].into_iter().map(String::from).collect());
}

#[test]
fn config_schema_rejects_unknown_fields() {
    // Matches this module's "never fail silently" stance: a typo'd or
    // not-yet-supported key must be a clear error, not silently dropped.
    let json = r#"{ "srcDirs": ["src"], "totallyMadeUpField": true }"#;
    let result: std::result::Result<DocgenConfigSchema, _> = serde_json::from_str(json);
    assert!(result.is_err(), "expected an unknown field to be rejected");
}

#[test]
fn config_schema_rejects_options_that_were_removed_for_doing_nothing() {
    // These were accepted and ignored before; a config still naming one must now say so instead of looking honored.
    for key in
        ["reactVersion", "crossPackage", "pandacssOutdir", "variantFunctions", "vanillaExtract", "resolveComplexTypes"]
    {
        let json = format!(r#"{{ "{key}": null }}"#);
        let err = serde_json::from_str::<DocgenConfigSchema>(&json).expect_err("a removed option must be rejected");
        assert!(err.to_string().contains(key), "expected the error to name `{key}`, got: {err}");
    }
}

#[test]
fn config_schema_rejects_a_typo_d_html_attributes_instead_of_defaulting() {
    let schema: DocgenConfigSchema = serde_json::from_str(r#"{ "htmlAttributes": "all" }"#).expect("valid JSON");

    let err = schema.into_pipeline_options().expect_err("all is not a recognized htmlAttributes");

    assert_eq!(err, "htmlAttributes is 'all', expected \"curated\", \"full\" or \"none\"");
}

#[test]
fn config_schema_defaults_are_used_when_a_field_is_absent() {
    let schema: DocgenConfigSchema = serde_json::from_str("{}").expect("empty object is valid");
    let opts = schema.into_pipeline_options().expect("an empty config has nothing to validate");
    let defaults = PipelineOptions::default();

    // Every field the schema maps, so one that starts defaulting differently is caught.
    assert_eq!(opts.src_dirs, defaults.src_dirs);
    assert_eq!(opts.exclude_patterns, defaults.exclude_patterns);
    assert_eq!(opts.exclude_prefixes, defaults.exclude_prefixes);
    assert_eq!(opts.html_attributes, defaults.html_attributes);
    assert_eq!(opts.tsconfig_path, defaults.tsconfig_path);
    assert_eq!(opts.cache_dir, defaults.cache_dir);
    assert_eq!(opts.extra_builtins, defaults.extra_builtins);
}

#[test]
fn try_load_config_maps_a_real_docgen_config_ts_file() {
    // Real end-to-end proof: an actual docgen.config.ts, evaluated by the real
    // node+tsx subprocess this module spawns, not just the pure schema mapping
    // tested above. Placed under apps/oracle/ (not a bare system tempdir) —
    // tsx is only a devDependency there, not hoisted to the repo root, and the
    // subprocess resolves it via Node's own ancestor-walking package resolution
    // starting from the config file's directory.
    let oracle_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/oracle");
    let tmp = tempfile::TempDir::new_in(&oracle_dir).unwrap();
    let config_path = tmp.path().join("docgen.config.ts");
    std::fs::write(
        &config_path,
        r#"
export default {
  srcDirs: ["src/components"],
  htmlAttributes: "full",
};
"#,
    )
    .unwrap();

    let opts = try_load_config(&config_path).expect("real config file should load successfully");
    let opts = opts.expect("expected Some(PipelineOptions)");

    assert_eq!(opts.src_dirs, vec![camino::Utf8PathBuf::from("src/components")]);
    assert_eq!(opts.html_attributes, HtmlAttributeMode::Full);
}

#[test]
fn try_load_config_resolves_a_relative_path_whose_parent_is_not_the_cwd() {
    // Found while validating SPEC-CLI-001d's AC-1: try_load_config changes
    // the node subprocess's cwd to the config file's parent (so the
    // subprocess resolves tsx/imports relative to the config, not to
    // wherever the CLI happened to be invoked from), but was passing the
    // *relative* input path through unchanged — node then re-resolved
    // that same relative string against the NEW cwd it had just been
    // given, not the cwd the caller meant it relative to. A relative
    // --config path whose parent directory differs from the process's
    // actual cwd failed with ERR_MODULE_NOT_FOUND even though the file
    // existed. Constructs a relative path without mutating the process's
    // actual cwd (cargo test's default cwd for this crate is
    // CARGO_MANIFEST_DIR, i.e. crates/cli — mirrors how `oracle_dir`
    // above is itself expressed relative to that same cwd) so this stays
    // safe under parallel test execution.
    let oracle_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/oracle");
    let tmp = tempfile::TempDir::new_in(&oracle_dir).unwrap();
    let config_path = tmp.path().join("docgen.config.ts");
    std::fs::write(&config_path, "export default { srcDirs: [\"src/components\"] };\n").unwrap();

    let tmp_dir_name = tmp.path().file_name().expect("tmp dir has a name");
    let relative_config_path = std::path::Path::new("../../apps/oracle").join(tmp_dir_name).join("docgen.config.ts");

    let opts = try_load_config(&relative_config_path)
        .expect("a relative --config path should resolve regardless of its parent directory")
        .expect("expected Some(PipelineOptions)");
    assert_eq!(opts.src_dirs, vec![camino::Utf8PathBuf::from("src/components")]);
}

// ── SPEC-CLI-001d AC-001: a --config path containing quote/backslash
// characters or the literal `');process.exit(1);('` sequence must not be
// interpretable as JavaScript syntax — the path is passed via the
// __DOCGEN_CONFIG_PATH env var, never concatenated into the script string
// handed to node. This is the one security-relevant criterion in the
// whole spec-drift review that had zero test coverage; the mechanism was
// previously verified by source inspection only.

#[test]
fn config_path_containing_quotes_and_an_exit_payload_is_not_interpreted_as_javascript() {
    let oracle_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/oracle");
    let tmp = tempfile::TempDir::new_in(&oracle_dir).unwrap();
    // Combines a single quote, a double quote, and the exact "break out of
    // a string literal and call process.exit(1)" payload AC-001 names —
    // if try_load_config ever regressed to string-concatenating this path
    // into the script source, node would execute process.exit(1) before
    // ever importing the real config, and this test would see the wrong
    // exit path (an Err, or Ok with the wrong content) instead of a clean
    // Ok(Some(..)) reflecting the config file's real content.
    let dangerous_name = "evil'\"');process.exit(1);('dir";
    let dangerous_dir = tmp.path().join(dangerous_name);
    std::fs::create_dir_all(&dangerous_dir).expect("create directory with a dangerous name");
    let config_path = dangerous_dir.join("docgen.config.ts");
    std::fs::write(&config_path, "export default { srcDirs: [\"src/components\"] };\n").unwrap();

    let opts = try_load_config(&config_path)
        .expect("a dangerous path must not be interpreted as JavaScript — no injected exit, no eval failure")
        .expect("expected Some(PipelineOptions), proving the real config content was evaluated");
    assert_eq!(
        opts.src_dirs,
        vec![camino::Utf8PathBuf::from("src/components")],
        "expected the actual config content, not something an injected statement could have produced"
    );
}

#[test]
fn try_load_config_surfaces_the_real_syntax_error_not_just_an_exit_status() {
    // Adversarial review finding: node/tsx's stderr was sent to
    // Stdio::null(), so a config file that failed to evaluate produced a
    // generic "failed to evaluate (exit status: 1)" with the actual
    // syntax error/stack trace thrown away — the one piece of
    // information that would actually tell the user what's wrong.
    let oracle_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/oracle");
    let tmp = tempfile::TempDir::new_in(&oracle_dir).unwrap();
    let config_path = tmp.path().join("docgen.config.ts");
    std::fs::write(&config_path, "export default {{{{ this is not valid javascript").unwrap();

    let err = try_load_config(&config_path).expect_err("a real syntax error should fail to evaluate");
    let message = format!("{err:?}");
    // This repo's apps/oracle uses tsx's esbuild-based transform, which
    // reports "ERROR: Expected identifier..."; a native TS-stripping
    // toolchain would instead say "SyntaxError" — accept either so this
    // isn't pinned to one specific transpiler's exact wording.
    assert!(
        message.contains("Expected identifier") || message.contains("SyntaxError"),
        "expected the real node/tsx error output in the message, got: {message}"
    );
}

#[test]
fn build_options_maps_extra_builtins_cli_flag() {
    let extra_builtins = vec!["Foo".to_string(), "Bar".to_string()];
    let opts = build_options(BuildOptionsArgs {
        src: &[],
        cache_dir: None,
        no_cache: false,
        html_attributes: None,
        config_path: None,
        extra_builtins: &extra_builtins,
        load_dependencies: false,
        no_load_dependencies: false,
        collapse_inherited_props: None,
    })
    .expect("build_options should succeed with no config file");

    let names: std::collections::BTreeSet<String> = opts.extra_builtins.iter().map(|s| s.to_string()).collect();
    assert_eq!(names, ["Bar", "Foo"].into_iter().map(String::from).collect());
}

#[test]
fn build_options_extra_builtins_cli_flag_overrides_config_file() {
    // Real end-to-end proof (same apps/oracle tempdir rationale as
    // try_load_config_maps_a_real_docgen_config_ts_file above): the CLI flag
    // must win over a config-file value, not merge with or lose to it.
    let oracle_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/oracle");
    let tmp = tempfile::TempDir::new_in(&oracle_dir).unwrap();
    let config_path = tmp.path().join("docgen.config.ts");
    std::fs::write(
        &config_path,
        r#"
export default {
  srcDirs: ["src/components"],
  extraBuiltins: ["FromConfig"],
};
"#,
    )
    .unwrap();

    let extra_builtins = vec!["FromCli".to_string()];
    let opts = build_options(BuildOptionsArgs {
        src: &[],
        cache_dir: None,
        no_cache: false,
        html_attributes: None,
        config_path: Some(config_path.to_str().unwrap()),
        extra_builtins: &extra_builtins,
        load_dependencies: false,
        no_load_dependencies: false,
        collapse_inherited_props: None,
    })
    .expect("build_options should succeed");

    let names: Vec<String> = opts.extra_builtins.iter().map(|s| s.to_string()).collect();
    assert_eq!(names, vec!["FromCli".to_string()]);
}

// ── SPEC-CLI-001a AC-018: a --config load failure returns Err from
// build_options (mapped to exit code 1 by main()'s shared Err path, same
// mechanism as AC-010/AC-013/AC-017) rather than any exit_code()-derived
// value. extract/check/inspect/watch all call build_options the same way
// (`build_options(...)?`), so this proves the shared mechanism once.

#[test]
fn build_options_propagates_a_config_load_failure_as_err() {
    let result = build_options(BuildOptionsArgs {
        src: &[],
        cache_dir: None,
        no_cache: false,
        html_attributes: None,
        config_path: Some("/nonexistent-parent-dir-xyz/docgen.config.ts"),
        extra_builtins: &[],
        load_dependencies: false,
        no_load_dependencies: false,
        collapse_inherited_props: None,
    });
    assert!(result.is_err(), "expected build_options to return Err for a failing --config path");
}

#[test]
fn config_schema_maps_every_field_onto_pipeline_options() {
    let json = r#"{
        "srcDirs": ["a", "b"],
        "excludePatterns": ["**/*.stories.tsx"],
        "excludePrefixes": ["Internal"],
        "htmlAttributes": "none",
        "tsconfigPath": "tsconfig.app.json",
        "cacheDir": ".cache/docgen",
        "extraBuiltins": ["Custom"]
    }"#;
    let schema: DocgenConfigSchema = serde_json::from_str(json).expect("valid config JSON");
    let opts = schema.into_pipeline_options().expect("valid config");

    assert_eq!(opts.src_dirs, vec![camino::Utf8PathBuf::from("a"), camino::Utf8PathBuf::from("b")]);
    assert_eq!(opts.exclude_patterns, vec!["**/*.stories.tsx".to_owned()]);
    assert_eq!(opts.exclude_prefixes, vec!["Internal".to_owned()]);
    assert_eq!(opts.html_attributes, HtmlAttributeMode::None);
    assert_eq!(opts.tsconfig_path, Some(camino::Utf8PathBuf::from("tsconfig.app.json")));
    assert_eq!(opts.cache_dir, Some(camino::Utf8PathBuf::from(".cache/docgen")));
    assert_eq!(opts.extra_builtins.iter().map(|s| s.to_string()).collect::<Vec<_>>(), vec!["Custom".to_owned()]);
}

#[test]
fn the_cache_is_on_unless_the_config_or_the_flag_turns_it_off() {
    let default: DocgenConfigSchema = serde_json::from_str("{}").expect("valid config JSON");
    assert!(default.into_pipeline_options().unwrap().use_cache);

    let config: DocgenConfigSchema = serde_json::from_str(r#"{ "cache": false }"#).expect("valid config JSON");
    assert!(!config.into_pipeline_options().unwrap().use_cache, "`cache: false` in the config");

    let flag = build_options(BuildOptionsArgs { no_cache: true, ..no_flags() }).expect("options");
    assert!(!flag.use_cache, "--no-cache");
    assert!(build_options(no_flags()).expect("options").use_cache, "no flag leaves it on");
}

#[test]
fn html_attributes_mode_names_map_onto_pipeline_options() {
    for (name, expected) in
        [("full", HtmlAttributeMode::Full), ("none", HtmlAttributeMode::None), ("curated", HtmlAttributeMode::Curated)]
    {
        let schema: DocgenConfigSchema =
            serde_json::from_str(&format!(r#"{{ "htmlAttributes": "{name}" }}"#)).expect("valid config JSON");
        assert_eq!(schema.into_pipeline_options().unwrap().html_attributes, expected, "htmlAttributes: {name}");
    }
}

fn no_flags<'a>() -> BuildOptionsArgs<'a> {
    BuildOptionsArgs {
        src: &[],
        cache_dir: None,
        no_cache: false,
        html_attributes: None,
        config_path: None,
        extra_builtins: &[],
        load_dependencies: false,
        no_load_dependencies: false,
        collapse_inherited_props: None,
    }
}

#[test]
fn build_options_applies_cli_flags_over_the_defaults() {
    let src = ["app".to_owned(), "lib".to_owned()];
    let extra_builtins = ["Custom".to_owned()];
    let opts = build_options(BuildOptionsArgs {
        src: &src,
        cache_dir: Some("cache"),
        html_attributes: Some("full"),
        extra_builtins: &extra_builtins,
        ..no_flags()
    })
    .unwrap();

    assert_eq!(opts.src_dirs, vec![camino::Utf8PathBuf::from("app"), camino::Utf8PathBuf::from("lib")]);
    assert_eq!(opts.cache_dir, Some(camino::Utf8PathBuf::from("cache")));
    assert_eq!(opts.html_attributes, HtmlAttributeMode::Full);
    assert_eq!(opts.extra_builtins.iter().map(|s| s.to_string()).collect::<Vec<_>>(), vec!["Custom".to_owned()]);
}

#[test]
fn build_options_maps_the_remaining_html_attribute_flag_values() {
    for (flag, expected) in [("none", HtmlAttributeMode::None), ("curated", HtmlAttributeMode::Curated)] {
        let opts = build_options(BuildOptionsArgs { html_attributes: Some(flag), ..no_flags() }).unwrap();
        assert_eq!(opts.html_attributes, expected, "--html-attributes {flag}");
    }
}

#[test]
fn build_options_rejects_an_unrecognized_html_attributes_flag_naming_the_value() {
    let err = build_options(BuildOptionsArgs { html_attributes: Some("all"), ..no_flags() })
        .expect_err("all is not a supported --html-attributes");
    assert_eq!(err.to_string(), "--html-attributes is 'all', which isn't a recognized value");
}

#[test]
fn load_dependencies_is_on_by_default_and_the_config_and_the_flags_set_it() {
    for (json, expected) in
        [("{}", true), (r#"{ "loadDependencies": true }"#, true), (r#"{ "loadDependencies": false }"#, false)]
    {
        let schema: DocgenConfigSchema = serde_json::from_str(json).expect("valid config JSON");
        assert_eq!(schema.into_pipeline_options().unwrap().load_dependencies, expected, "{json}");
    }
    assert!(build_options(no_flags()).unwrap().load_dependencies);
    assert!(build_options(BuildOptionsArgs { load_dependencies: true, ..no_flags() }).unwrap().load_dependencies);
    assert!(!build_options(BuildOptionsArgs { no_load_dependencies: true, ..no_flags() }).unwrap().load_dependencies);
}

#[test]
fn the_experimental_semantic_backend_is_off_unless_the_config_turns_it_on() {
    let options = |json: &str| {
        let schema: DocgenConfigSchema = serde_json::from_str(json).expect("valid config JSON");
        schema.into_pipeline_options().unwrap()
    };

    let off = options("{}");
    assert!(!off.experimental_semantic_types);
    assert_eq!(off.semantic_tsc_path, None);

    let on = options(r#"{ "experimentalSemanticTypes": true, "tscPath": "node_modules/typescript/bin/tsc" }"#);
    assert!(on.experimental_semantic_types);
    assert_eq!(on.semantic_tsc_path, Some("node_modules/typescript/bin/tsc".into()));
}

#[test]
fn collapse_inherited_props_comes_from_the_config_or_the_flag_and_names_a_bad_value() {
    for (json, expected) in [
        ("{}", CollapseInheritedProps::Dependencies),
        (r#"{ "collapseInheritedProps": "all" }"#, CollapseInheritedProps::All),
        (r#"{ "collapseInheritedProps": "none" }"#, CollapseInheritedProps::None),
    ] {
        let schema: DocgenConfigSchema = serde_json::from_str(json).expect("valid config JSON");
        assert_eq!(schema.into_pipeline_options().unwrap().collapse_inherited_props, expected, "{json}");
    }
    let schema: DocgenConfigSchema = serde_json::from_str(r#"{ "collapseInheritedProps": "some" }"#).unwrap();
    assert_eq!(
        schema.into_pipeline_options().unwrap_err(),
        "collapseInheritedProps is 'some', expected \"dependencies\", \"all\" or \"none\""
    );
    let flag = BuildOptionsArgs { collapse_inherited_props: Some(CollapseInheritedProps::None), ..no_flags() };
    assert_eq!(build_options(flag).unwrap().collapse_inherited_props, CollapseInheritedProps::None);
}

#[test]
fn load_config_file_stops_at_a_workspace_marker_without_reading_configs_above_it() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Would fail to evaluate if it were ever loaded.
    std::fs::write(tmp.path().join("docgen.config.ts"), "throw new Error('must not be loaded');\n").unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("pnpm-workspace.yaml"), "packages: []\n").unwrap();
    let package = workspace.join("packages");
    std::fs::create_dir(&package).unwrap();

    assert!(load_config_file(&package).unwrap().is_none());
}

/// A `docgen.config.ts` with `source`, placed under apps/oracle so `tsx` resolves (see the real-file test above).
fn write_config(source: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let oracle_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/oracle");
    let tmp = tempfile::TempDir::new_in(oracle_dir).unwrap();
    let path = tmp.path().join("docgen.config.ts");
    std::fs::write(&path, source).unwrap();
    let path = path.canonicalize().unwrap();
    (tmp, path)
}

#[test]
fn a_config_of_the_wrong_shape_is_an_error_naming_the_file() {
    let (_tmp, path) = write_config("export default { srcDirs: 42 };\n");

    let err = try_load_config(&path).expect_err("srcDirs must be a list of strings");

    assert!(
        err.to_string()
            .starts_with(&format!("docgen.config.ts at {} doesn't match the expected shape: ", path.display())),
        "{err}"
    );
}

#[test]
fn a_config_with_an_unrecognized_html_attributes_is_an_error_naming_the_file_and_value() {
    let (_tmp, path) = write_config("export default { htmlAttributes: 'all' };\n");

    let err = try_load_config(&path).expect_err("all is not a supported htmlAttributes");

    assert_eq!(
        err.to_string(),
        format!(
            "docgen.config.ts at {}: htmlAttributes is 'all', expected \"curated\", \"full\" or \"none\"",
            path.display()
        )
    );
}
