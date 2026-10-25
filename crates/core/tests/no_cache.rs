//! `PipelineOptions::use_cache` off: nothing is read from or written to the cache directory, and the output is the same.

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::{Utf8Path, Utf8PathBuf};

fn project() -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    for (path, body) in [
        ("node_modules/@types/react/package.json", r#"{ "name": "@types/react", "version": "19.0.0", "types": "index.d.ts" }"#),
        ("node_modules/@types/react/index.d.ts", "export = React;\nexport as namespace React;\ndeclare namespace React { type ReactNode = string | null; }\n"),
        ("node_modules/pkg-a/package.json", r#"{ "name": "pkg-a", "version": "1.0.0", "types": "index.d.ts" }"#),
        ("node_modules/pkg-a/index.d.ts", "export interface AProps { a?: string }\n"),
        ("src/A.tsx", "import type { AProps } from 'pkg-a';\nexport function A(props: AProps) { return null; }\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    (tmp, root)
}

fn options(root: &Utf8Path, cache: &Utf8Path, use_cache: bool) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![root.join("src")],
        cache_dir: Some(cache.to_owned()),
        load_dependencies: true,
        use_cache,
        ..Default::default()
    }
}

fn files_in(dir: &Utf8Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> =
        entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn a_run_with_the_cache_off_reads_and_writes_nothing_and_publishes_the_same() {
    let (_tmp, root) = project();
    let cache = root.join("cache");

    let off = extract(&options(&root, &cache, false));
    assert_eq!(files_in(&cache), Vec::<String>::new(), "no cache file is written");
    assert!(!cache.exists(), "not even the directory");

    // Warm a cache, then run with the cache off over it: the cache on disk is not read either.
    let on = extract(&options(&root, &cache, true));
    let written = files_in(&cache);
    assert!(written.iter().any(|name| name.ends_with(".msgpack")), "the dependency index is written: {written:?}");
    assert!(written.iter().any(|name| name == "manifest.json"), "the parse cache is written: {written:?}");

    let off_again = extract(&options(&root, &cache, false));
    assert_eq!(files_in(&cache), written, "the cache directory is left as it was");
    assert_eq!(off_again.stats.dts_cache_hits, 0, "a warm cache on disk is not used");

    assert_eq!(off.components, on.components);
    assert_eq!(off.components, off_again.components);
    assert_eq!(off.components.keys().collect::<Vec<_>>(), ["A"]);
    assert_eq!(off.components["A"].props.keys().collect::<Vec<_>>(), ["a"]);
}
