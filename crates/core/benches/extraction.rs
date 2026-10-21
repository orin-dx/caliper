#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

fn fixture_dir(fixture: &str) -> camino::Utf8PathBuf {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest_dir
        .join("../../fixtures")
        .join(std::path::Path::new(fixture).parent().unwrap_or(std::path::Path::new(".")));
    camino::Utf8PathBuf::from_path_buf(dir).expect("valid utf8 path")
}

fn extract_file_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract_file");
    for fixture in ["shadcn/button.tsx", "shadcn/input.tsx", "radix/button.d.ts", "mui/Button.d.ts"] {
        let options = caliper_docgen_core::pipeline::PipelineOptions {
            src_dirs: vec![fixture_dir(fixture)],
            ..Default::default()
        };
        group.bench_with_input(BenchmarkId::from_parameter(fixture), &options, |b, options| {
            b.iter(|| caliper_docgen_core::pipeline::extract(options));
        });
    }
    group.finish();
}

/// Single-file incremental update cost via `WatchSession`, vs. the cold
/// extraction it's compared against above. RDT and react-docgen have no
/// incremental API — a real edit there always re-pays the cold cost, so this
/// number is what this tool saves per edit that neither comparator can.
fn incremental_update_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_update");
    for fixture in ["shadcn/button.tsx", "mantine", "mui"] {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture_path = manifest_dir.join("../../fixtures").join(fixture);
        let (lib_dir, changed_file) = if fixture_path.is_dir() {
            let first = std::fs::read_dir(&fixture_path)
                .expect("fixture dir exists")
                .filter_map(Result::ok)
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "ts" || e == "tsx"))
                .expect("at least one .ts/.tsx file in fixture dir");
            (fixture_path, first)
        } else {
            (fixture_path.parent().expect("file has a parent dir").to_path_buf(), fixture_path)
        };

        let options = caliper_docgen_core::pipeline::PipelineOptions {
            src_dirs: vec![camino::Utf8PathBuf::from_path_buf(lib_dir).expect("valid utf8 path")],
            ..Default::default()
        };
        let changed = camino::Utf8PathBuf::from_path_buf(changed_file).expect("valid utf8 path");

        let session = caliper_docgen_core::pipeline::WatchSession::new(options);
        let _ignored = session.initialize();

        group.bench_with_input(BenchmarkId::from_parameter(fixture), &changed, |b, changed| {
            b.iter(|| session.update_file(changed));
        });
    }
    group.finish();
}

criterion_group!(benches, extract_file_bench, incremental_update_bench);
criterion_main!(benches);
