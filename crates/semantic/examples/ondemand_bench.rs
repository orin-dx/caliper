#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]
//! What asking `tsc` for one component at a time costs, against asking for the whole project once (ADR 0022).
//!
//!   ondemand_bench --src DIR --tsc PATH [--tsconfig FILE] [--mode on-demand|project] [--count N] [--seed S] [--batch K]
//!
//! One scenario per process, so `/usr/bin/time -l` reports the checker's peak RSS (the largest descendant). Prints one
//! JSON line: `structuralMs`, then for `project` the single `projectMs`, or for `on-demand` `startMs` (spawn and
//! handshake), `firstMs` (the first component, with a cold program) and `eachMs` (the rest, sorted), plus how many
//! components the checker answered and the props it found against the structural pass.
use std::time::{Duration, Instant};

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions};
use caliper_docgen_core::types::semantic::SemanticProvider;
use caliper_docgen_semantic::{resolve, targets_of, OnDemand, OnDemandConfig, SemanticRequest};
use camino::Utf8PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|at| args.get(at + 1)).cloned()
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = Utf8PathBuf::from(arg(&args, "--src").expect("--src"));
    let tsc = Utf8PathBuf::from(arg(&args, "--tsc").expect("--tsc"));
    let tsconfig = arg(&args, "--tsconfig").map(Utf8PathBuf::from);
    let mode = arg(&args, "--mode").unwrap_or_else(|| "on-demand".to_string());
    let count: usize = arg(&args, "--count").map_or(50, |n| n.parse().expect("--count"));
    let batch: usize = arg(&args, "--batch").map_or(1, |n| n.parse().expect("--batch"));
    let mut seed: u64 = arg(&args, "--seed").map_or(1, |n| n.parse().expect("--seed")) | 1;

    let options = PipelineOptions {
        src_dirs: vec![src.clone()],
        load_dependencies: true,
        html_attributes: HtmlAttributeMode::Curated,
        ..Default::default()
    };
    let started = Instant::now();
    let output = extract(&options);
    let structural_ms = ms(started.elapsed());
    let all = targets_of(&output);
    let structural_props: usize = output.components.values().map(|entry| entry.props.len()).sum();
    let root = std::fs::canonicalize(&src).ok().and_then(|p| Utf8PathBuf::from_path_buf(p).ok()).unwrap_or(src);

    if mode == "project" {
        let mut request = SemanticRequest::new(root, all.clone());
        request.tsconfig = tsconfig;
        request.tsc_path = Some(tsc);
        let started = Instant::now();
        let outcome = resolve(request);
        println!(
            "{{\"mode\":\"project\",\"components\":{},\"structuralMs\":{structural_ms:.1},\"projectMs\":{:.1},\"answered\":{},\"props\":{}}}",
            all.len(),
            ms(started.elapsed()),
            outcome.components.len(),
            outcome.components.iter().map(|c| c.props.len()).sum::<usize>(),
        );
        return;
    }

    let mut config = OnDemandConfig::new(root);
    config.tsconfig = tsconfig;
    config.tsc_path = Some(tsc);
    let started = Instant::now();
    let checker = OnDemand::new(config);
    checker.warm_up();
    // A request queued behind the warm-up returns when the process is up and the first program is built.
    let mut picks: Vec<_> = Vec::new();
    let exported: Vec<_> = all.iter().filter(|t| t.exported).cloned().collect();
    for _ in 0..count.min(exported.len()) {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        picks.push(exported[(seed as usize) % exported.len()].clone());
    }
    let mut times = Vec::new();
    let mut answered = 0usize;
    let mut found_props = 0usize;
    let mut structural_of_answered = 0usize;
    let mut fallbacks = 0usize;
    let mut unavailable = String::new();
    for group in picks.chunks(batch.max(1)) {
        let one = Instant::now();
        let outcome = checker.request(group.to_vec()).recv_timeout(Duration::from_secs(120));
        // Per component, so batches of different sizes compare.
        times.push(ms(one.elapsed()) / group.len() as f64);
        match outcome {
            Ok(outcome) => {
                for diagnostic in &outcome.diagnostics {
                    unavailable = diagnostic.message.clone();
                }
                fallbacks += outcome.fallbacks.len();
                for component in &outcome.components {
                    answered += 1;
                    found_props += component.props.len();
                    structural_of_answered += output
                        .components
                        .values()
                        .find(|e| e.file_path == component.file_path && e.display_name == component.display_name)
                        .map_or(0, |e| e.props.len());
                }
            }
            Err(error) => unavailable = format!("no answer: {error}"),
        }
    }
    let total_ms = ms(started.elapsed());
    let first_ms = times.first().copied().unwrap_or(0.0);
    let mut rest: Vec<f64> = times.iter().skip(1).copied().collect();
    rest.sort_by(|a, b| a.total_cmp(b));
    println!(
        "{{\"mode\":\"on-demand\",\"components\":{},\"asked\":{},\"structuralMs\":{structural_ms:.1},\"firstMs\":{first_ms:.1},\"totalMs\":{total_ms:.1},\"p50\":{:.1},\"p90\":{:.1},\"max\":{:.1},\"answered\":{answered},\"fallbacks\":{fallbacks},\"propsChecker\":{found_props},\"propsStructuralSame\":{structural_of_answered},\"structuralProps\":{structural_props},\"note\":{:?}}}",
        all.len(),
        picks.len(),
        percentile(&rest, 0.5),
        percentile(&rest, 0.9),
        rest.last().copied().unwrap_or(0.0),
        unavailable,
    );
}
