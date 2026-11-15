use super::*;

fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    for (file, body) in files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    (tmp, root)
}

fn file_of(resolution: ModuleResolution) -> Option<Utf8PathBuf> {
    match resolution {
        ModuleResolution::File(path) | ModuleResolution::PackageTypes(path) => Some(path),
        ModuleResolution::Unresolved(_) => None,
    }
}

#[test]
fn resolvers_made_after_an_edit_see_it() {
    let (_tmp, root) = project(&[("src/a.ts", "import './b';\n")]);
    let importer = root.join("src/a.ts");
    let options = PipelineOptions::default();
    assert_eq!(file_of(ModuleResolvers::new(&options).resolve("./b", &importer)), None);

    std::fs::write(root.join("src/b.ts"), "export {};\n").unwrap();

    assert_eq!(file_of(ModuleResolvers::new(&options).resolve("./b", &importer)), Some(root.join("src/b.ts")));
}

#[test]
fn a_package_manifest_edit_moves_the_declarations() {
    let (_tmp, root) = project(&[
        ("node_modules/lib/package.json", r#"{ "types": "./a.d.ts" }"#),
        ("node_modules/lib/a.d.ts", "export {};\n"),
        ("node_modules/lib/b.d.ts", "export {};\n"),
    ]);
    let importer = root.join("index.ts");
    let first = ModuleResolvers::new(&PipelineOptions::default()).package_declaration(&importer, "lib");
    assert_eq!(first.as_deref(), Some(root.join("node_modules/lib/a.d.ts").as_str()));

    std::fs::write(root.join("node_modules/lib/package.json"), r#"{ "types": "./b.d.ts" }"#).unwrap();

    let second = ModuleResolvers::new(&PipelineOptions::default()).package_declaration(&importer, "lib");
    assert_eq!(second.as_deref(), Some(root.join("node_modules/lib/b.d.ts").as_str()));
}

#[test]
fn paths_and_custom_conditions_come_from_the_nearest_tsconfig() {
    let (_tmp, root) = project(&[
        (
            "tsconfig.json",
            r#"{ // a comment
                "compilerOptions": { "baseUrl": ".", "paths": { "@ui/*": ["src/ui/*"] }, "customConditions": ["source"], },
            }"#,
        ),
        ("src/ui/button.ts", "export {};\n"),
        ("src/app.ts", "import '@ui/button';\n"),
        (
            "node_modules/ws/package.json",
            r#"{ "exports": { ".": { "source": "./src/index.ts", "types": "./dist/index.d.ts" } } }"#,
        ),
        ("node_modules/ws/src/index.ts", "export {};\n"),
        ("node_modules/ws/dist/index.d.ts", "export {};\n"),
    ]);
    let resolvers = ModuleResolvers::new(&PipelineOptions::default());
    let importer = root.join("src/app.ts");

    assert_eq!(file_of(resolvers.resolve("@ui/button", &importer)), Some(root.join("src/ui/button.ts")));
    assert_eq!(file_of(resolvers.resolve("ws", &importer)), Some(root.join("node_modules/ws/src/index.ts")));
}

#[test]
fn many_resolvers_in_a_row_each_free_their_region() {
    let (_tmp, root) = project(&[("src/a.ts", ""), ("src/b.ts", "")]);
    for _ in 0..50 {
        let resolvers = ModuleResolvers::new(&PipelineOptions::default());
        assert!(file_of(resolvers.resolve("./b", &root.join("src/a.ts"))).is_some());
    }
}

#[cfg(unix)]
#[test]
fn a_project_file_reached_through_a_relative_symlink_resolves_to_its_real_path() {
    let (_tmp, root) = project(&[("src/real/types.ts", "export {};\n"), ("src/app.ts", "import './link/types';\n")]);
    std::os::unix::fs::symlink("real", root.join("src/link")).unwrap();
    let resolvers = ModuleResolvers::new(&PipelineOptions::default());

    let through_link = file_of(resolvers.resolve("./link/types", &root.join("src/app.ts")));
    let direct = file_of(resolvers.resolve("./real/types", &root.join("src/app.ts")));

    assert_eq!(through_link, Some(root.join("src/real/types.ts")));
    assert_eq!(through_link, direct);
}

#[test]
fn a_commonjs_importer_finds_a_package_that_declares_only_its_require_types() {
    let (_tmp, root) = project(&[
        (
            "node_modules/rq/package.json",
            r#"{ "exports": { ".": { "require": { "types": "./index.d.cts", "default": "./index.cjs" } } } }"#,
        ),
        ("node_modules/rq/index.d.cts", "export {};\n"),
        ("node_modules/rq/index.cjs", "module.exports = {};\n"),
        (
            "node_modules/im/package.json",
            r#"{ "exports": { ".": { "import": { "types": "./index.d.mts", "default": "./index.mjs" } } } }"#,
        ),
        ("node_modules/im/index.d.mts", "export {};\n"),
        ("node_modules/im/index.mjs", "export {};\n"),
        ("src/a.cts", ""),
        ("src/a.mts", ""),
        ("src/a.ts", ""),
    ]);
    let resolvers = ModuleResolvers::new(&PipelineOptions::default());
    let require_types = Some(root.join("node_modules/rq/index.d.cts"));
    let import_types = Some(root.join("node_modules/im/index.d.mts"));

    assert_eq!(file_of(resolvers.resolve("rq", &root.join("src/a.cts"))), require_types);
    assert_eq!(file_of(resolvers.resolve("im", &root.join("src/a.cts"))), import_types);
    assert_eq!(file_of(resolvers.resolve("im", &root.join("src/a.mts"))), import_types);
    assert_eq!(file_of(resolvers.resolve("im", &root.join("src/a.ts"))), import_types);
    assert_eq!(file_of(resolvers.resolve("rq", &root.join("src/a.ts"))), require_types);
}

#[test]
fn a_relative_configured_tsconfig_is_read_from_the_working_directory() {
    let tmp = tempfile::Builder::new().prefix("rel-tsconfig-").tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    std::fs::create_dir_all(root.join("src/ui")).unwrap();
    std::fs::write(root.join("src/ui/x.ts"), "export {};\n").unwrap();
    std::fs::write(root.join("src/a.ts"), "").unwrap();
    std::fs::write(
        root.join("tsconfig.app.json"),
        r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@ui/*": ["src/ui/*"] } } }"#,
    )
    .unwrap();
    let relative = Utf8PathBuf::from(tmp.path().file_name().unwrap().to_str().unwrap()).join("tsconfig.app.json");
    let options = PipelineOptions { tsconfig_path: Some(relative.clone()), ..Default::default() };
    let resolvers = ModuleResolvers::new(&options);

    assert_eq!(file_of(resolvers.resolve("@ui/x", &root.join("src/a.ts"))), Some(root.join("src/ui/x.ts")));
    assert_eq!(resolvers.tsconfig_error(&relative), None);
}

#[test]
fn the_referenced_project_that_includes_a_file_governs_it() {
    let (_tmp, root) = project(&[
        ("tsconfig.json", r#"{ "files": [], "references": [{ "path": "./tsconfig.app.json" }] }"#),
        (
            "tsconfig.app.json",
            r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@ui/*": ["src/ui/*"] } }, "include": ["src"] }"#,
        ),
        ("src/ui/x.ts", "export {};\n"),
        ("src/a.ts", ""),
    ]);
    let resolvers = ModuleResolvers::new(&PipelineOptions::default());

    assert_eq!(file_of(resolvers.resolve("@ui/x", &root.join("src/a.ts"))), Some(root.join("src/ui/x.ts")));
}

#[test]
fn without_clearing_the_file_system_cache_a_new_file_stays_invisible() {
    // The control for `resolvers_made_after_an_edit_see_it`: it passes because `new` drops the cache. Without that,
    // the next revision would answer from the first one's view of the disk.
    let (_tmp, root) = project(&[("src/a.ts", "")]);
    let importer = root.join("src/a.ts");
    let options = PipelineOptions::default();
    assert_eq!(file_of(ModuleResolvers::new(&options).resolve("./b", &importer)), None);

    std::fs::write(root.join("src/b.ts"), "export {};\n").unwrap();

    assert_eq!(file_of(ModuleResolvers::keeping_file_system_cache(&options).resolve("./b", &importer)), None);
    assert_eq!(file_of(ModuleResolvers::new(&options).resolve("./b", &importer)), Some(root.join("src/b.ts")));
}
