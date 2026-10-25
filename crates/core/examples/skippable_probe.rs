#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]
//! What a byte-scan prefilter could save on a source directory: how much of the parse time goes to files that give
//! resolution nothing (`pipeline::probe`).
//!
//!   cargo run --release -p caliper-docgen-core --features probe --example skippable_probe -- --src DIR
//!
//! Prints one JSON line (see `ProbeReport`). The parse cache is off, so every run is cold.
use caliper_docgen_core::pipeline::{probe::skippable_probe, PipelineOptions};
use camino::Utf8PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = args.iter().position(|a| a == "--src").and_then(|at| args.get(at + 1)).expect("--src DIR");
    let options = PipelineOptions { src_dirs: vec![Utf8PathBuf::from(src)], ..Default::default() };
    println!("{}", serde_json::to_string(&skippable_probe(&options)).expect("serialize"));
}
