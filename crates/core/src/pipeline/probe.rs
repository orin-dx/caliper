//! What a byte-scan prefilter could save: the time spent parsing files that give resolution nothing. Behind the `probe`
//! feature for `examples/skippable_probe.rs`; not part of the library.
use std::time::Instant;

use rayon::prelude::*;
use serde::Serialize;

use super::discover::discover_files;
use super::{extract, PipelineOptions};
use crate::types::collected::{LexedExport, SourceData};

/// One cold run over a source directory. Thread times sum what each file took on its thread, so they exceed wall time
/// by up to the thread count.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    pub threads: usize,
    pub files: usize,
    pub skippable_files: usize,
    pub discover_ms: f64,
    pub read_thread_ms: f64,
    pub parse_wall_ms: f64,
    pub parse_thread_ms: f64,
    pub skippable_parse_thread_ms: f64,
    /// The whole `extract` (discover, parse, merge, resolve), cold, for the denominators.
    pub extract_wall_ms: f64,
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Whether anything resolution reads comes out of the file. Imports do not count: they matter only to a file that has
/// something else to resolve, and neither does a plain `export const x`; a re-export or a renamed export does.
fn contributes(data: &SourceData) -> bool {
    let plain_exports_only =
        data.exports.iter().all(|export| matches!(export, LexedExport::LocalDeclaration { declared_as: None, .. }));
    !(data.interfaces.is_empty()
        && data.type_aliases.is_empty()
        && data.enums.is_empty()
        && data.const_arrays.is_empty()
        && data.component_mappings.is_empty()
        && data.function_results.is_empty()
        && data.type_param_defaults.is_empty()
        && plain_exports_only
        && data.diagnostics.is_empty())
}

/// Parses every file `options` discovers, then runs `extract` once, timing both.
pub fn skippable_probe(options: &PipelineOptions) -> ProbeReport {
    let discover = Instant::now();
    let (files, _) = discover_files(&options.src_dirs, &options.exclude_patterns);
    let discover_ms = ms(discover);

    let parse = Instant::now();
    let per_file: Vec<(f64, f64, bool)> = files
        .par_iter()
        .filter_map(|path| {
            let read = Instant::now();
            let source = std::fs::read_to_string(path).ok()?;
            let read_ms = ms(read);
            let parsed = Instant::now();
            let data = crate::extractor::parse_file(path, &source);
            Some((read_ms, ms(parsed), !contributes(&data)))
        })
        .collect();
    let parse_wall_ms = ms(parse);

    let extract_start = Instant::now();
    drop(extract(options));
    let extract_wall_ms = ms(extract_start);

    ProbeReport {
        threads: rayon::current_num_threads(),
        files: per_file.len(),
        skippable_files: per_file.iter().filter(|file| file.2).count(),
        discover_ms,
        read_thread_ms: per_file.iter().map(|file| file.0).sum(),
        parse_wall_ms,
        parse_thread_ms: per_file.iter().map(|file| file.1).sum(),
        skippable_parse_thread_ms: per_file.iter().filter(|file| file.2).map(|file| file.1).sum(),
        extract_wall_ms,
    }
}
