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
