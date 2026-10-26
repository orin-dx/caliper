#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]
//! Session start and time-to-answer for an eager session and a lazy one (ADR 0016), one scenario per process so
//! `/usr/bin/time -l` reports that scenario's peak RSS.
//!
//!   lazy_bench --src DIR --cache DIR [--eager] [--dependencies] [--full] [--count N] [--seed S] [--parallel]
//!
//! Prints one JSON line: `startMs` (construct + initialize), `keysMs` (listing keys, lazy only), `answerMs` (resolving
//! `count` seeded-random components after start; 0 for an eager session, which has resolved them all), `eachMs` (the
//! serial per-component times, sorted), and `digest` (a hash of the answers, to compare scenarios).
use std::hash::{Hash, Hasher};
use std::time::Instant;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, WatchSession};
use camino::Utf8PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|at| args.get(at + 1)).cloned()
}

fn digest(answer: &impl std::fmt::Debug) -> u64 {
    let mut hasher = rustc_hash::FxHasher::default();
    format!("{answer:?}").hash(&mut hasher);
    hasher.finish()
}

/// With `--dump DIR`, each answer in full, to diff two scenarios.
fn dump(args: &[String], key: &str, answer: &impl std::fmt::Debug) {
    if let Some(dir) = arg(args, "--dump") {
        std::fs::create_dir_all(&dir).expect("--dump");
        std::fs::write(format!("{dir}/{}.txt", key.replace('/', "_")), format!("{answer:#?}")).expect("--dump");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let src = arg(&args, "--src").expect("--src");
    let cache = arg(&args, "--cache").expect("--cache");
    let count: usize = arg(&args, "--count").map_or(0, |n| n.parse().expect("--count"));
    let mut seed: u64 = arg(&args, "--seed").map_or(1, |n| n.parse().expect("--seed")) | 1;
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from(src)],
        cache_dir: Some(Utf8PathBuf::from(cache)),
        load_dependencies: flag("--dependencies"),
        html_attributes: if flag("--full") { HtmlAttributeMode::Full } else { HtmlAttributeMode::Curated },
        ..Default::default()
    };

    let start = Instant::now();
    let lazy = !flag("--eager");
    let session = if lazy { WatchSession::lazy(options) } else { WatchSession::new(options) };
    drop(session.initialize());
    let start_ms = start.elapsed().as_secs_f64() * 1000.0;

    let keys_start = Instant::now();
    let keys = session.component_keys();
    let keys_ms = keys_start.elapsed().as_secs_f64() * 1000.0;

    if let Some(path) = arg(&args, "--write-keys") {
        std::fs::write(path, keys.join("\n")).expect("--write-keys");
    }
    // A lazy session with dependencies lists re-exported imported components it has not checked; pick from the keys a
    // batch run published so every scenario answers the same components.
    let keys: Vec<String> = match arg(&args, "--keys") {
        Some(path) => std::fs::read_to_string(path).expect("--keys").lines().map(str::to_owned).collect(),
        None => keys,
    };
    let mut picked = keys.clone();
    for i in (1..picked.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        picked.swap(i, (seed % (i as u64 + 1)) as usize);
    }
    // `--all`: every component, in file order, as a static build asks.
    if flag("--all") {
        picked = keys.clone();
    } else {
        picked.truncate(count);
    }
    if let Some(names) = arg(&args, "--pick") {
        picked = names.split(',').map(str::to_owned).collect();
    }

    let mut hasher = rustc_hash::FxHasher::default();
    let mut each_ms: Vec<f64> = Vec::new();
    let mut per_key: Vec<(String, u64)> = Vec::new();
    let answer_start = Instant::now();
    if lazy && flag("--parallel") {
        for answer in session.components(&picked) {
            format!("{answer:?}").hash(&mut hasher);
        }
    } else if lazy {
        for key in &picked {
            let one = Instant::now();
            let answer = session.component(key);
            each_ms.push(one.elapsed().as_secs_f64() * 1000.0);
            format!("{answer:?}").hash(&mut hasher);
            per_key.push((key.clone(), digest(&answer)));
            dump(&args, key, &answer);
        }
    } else {
        // An eager session has resolved everything; reading answers back is a lookup.
        for key in &picked {
            let answer = session.component(key);
            format!("{answer:?}").hash(&mut hasher);
            per_key.push((key.clone(), digest(&answer)));
            dump(&args, key, &answer);
        }
    }
    let answer_ms = if lazy { answer_start.elapsed().as_secs_f64() * 1000.0 } else { 0.0 };
    each_ms.sort_by(f64::total_cmp);
    if let Some(path) = arg(&args, "--digests") {
        let lines: Vec<String> = per_key.iter().map(|(key, digest)| format!("{key}\t{digest:016x}")).collect();
        std::fs::write(path, lines.join("\n")).expect("--digests");
    }
    let stats = session.lazy_stats();
    println!(
        "{{\"lazy\":{lazy},\"components\":{},\"count\":{},\"startMs\":{start_ms:.1},\"keysMs\":{keys_ms:.1},\"answerMs\":{answer_ms:.1},\"eachMs\":{:?},\"resolved\":{},\"dependencyFiles\":{},\"digest\":\"{:016x}\"}}",
        keys.len(),
        picked.len(),
        each_ms.iter().map(|ms| (ms * 10.0).round() / 10.0).collect::<Vec<_>>(),
        stats.map_or(0, |s| s.resolved),
        stats.map_or(0, |s| s.dependency_files),
        hasher.finish(),
    );
}
