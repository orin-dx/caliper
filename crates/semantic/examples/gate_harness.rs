#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, clippy::print_stderr)]
//! The two measurable ADR 0026 gates before the checker becomes the default, through the session a Storybook addon runs:
//! a lazy `WatchSession` asking a warm `OnDemand` over the project's own `tsc` (`just gate-measure` runs both).
//!
//!   gate_harness memory      --src DIR --tsc PATH [--edits 650] [--max-rss-mib N]
//!   gate_harness determinism --src DIR --tsc PATH [--components 150] [--threads 1,2,4] [--runs 3]
//!
//! `memory` rewrites one component's file, tells the session, and asks for the component, `--edits` times; it samples the
//! checker's PID and resident memory after every answer and prints the recycles, the peak, and whether the last answer
//! equals a fresh session's. `determinism` asks a fresh checker for the same components under each `RAYON_NUM_THREADS`
//! (tsrs's `--api` has no checker-count flag; see docs/gate-measurements.md), `--runs` times each, alternating the query
//! order, and byte-compares every output. Each prints one JSON line and exits 1 on a failed check.
use std::collections::BTreeSet;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use caliper_docgen_core::pipeline::{CollapseInheritedProps, ComponentAnswer, PipelineOptions, WatchSession};
use caliper_docgen_core::types::semantic::SemanticProvider;
use caliper_docgen_semantic::{OnDemand, OnDemandConfig};
use camino::Utf8PathBuf;

const CHECKER_NOTE: &str = "resolved by the TypeScript checker";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|at| args.get(at + 1)).cloned()
}

/// A lazy session with a warm checker, configured as the addon configures it (dependencies loaded, nothing collapsed).
fn open(
    src: &Utf8PathBuf,
    tsc: &str,
    env: Vec<(String, String)>,
    max_rss_mib: Option<u64>,
) -> (WatchSession, Arc<OnDemand>) {
    let mut config = OnDemandConfig::new(src.clone());
    config.tsconfig = src.ancestors().map(|dir| dir.join("tsconfig.json")).find(|file| file.is_file());
    config.tsc_path = Some(Utf8PathBuf::from(tsc));
    config.request_timeout = Duration::from_secs(120);
    config.extra_env = env;
    config.max_rss_mib = max_rss_mib.unwrap_or(config.max_rss_mib);
    let checker = Arc::new(OnDemand::new(config));
    let options = PipelineOptions {
        src_dirs: vec![src.clone()],
        load_dependencies: true,
        collapse_inherited_props: CollapseInheritedProps::None,
        ..PipelineOptions::default()
    };
    let session = WatchSession::lazy(options).with_semantic(checker.clone(), Duration::from_secs(120));
    drop(session.initialize());
    (session, checker)
}

fn by_checker(answer: &ComponentAnswer) -> bool {
    answer.diagnostics.iter().any(|d| d.message.contains(CHECKER_NOTE))
}

/// Resident MiB and thread count of a process, from `ps`; `None` once it has gone.
fn process_stats(pid: u32) -> Option<(u64, usize)> {
    let ps = |args: &[&str]| {
        let out = Command::new("ps").args(args).arg("-p").arg(pid.to_string()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let rss: u64 = ps(&["-o", "rss="])?.trim().parse().ok()?;
    Some((rss / 1024, ps(&["-M"])?.lines().count().saturating_sub(1)))
}

fn edit_source(n: usize) -> String {
    format!(
        "import type {{ ButtonProps }} from '@chakra-ui/react'\nexport function GateEdited(props: ButtonProps & {{ edit{n}: string }}) {{\n  return null\n}}\n"
    )
}

fn memory(src: &Utf8PathBuf, tsc: &str, edits: usize, max_rss_mib: Option<u64>) -> bool {
    let file = src.join("GateEdited.tsx");
    std::fs::write(&file, edit_source(0)).unwrap();
    let (session, checker) = open(src, tsc, Vec::new(), max_rss_mib);
    let (mut processes, mut peak, mut answered, mut stale) = (BTreeSet::new(), 0, 0, 0);
    for n in 1..=edits {
        std::fs::write(&file, edit_source(n)).unwrap();
        session.update_file(&file);
        let answer = session.component("GateEdited").expect("GateEdited is a component");
        // The answer must be the checker's, for this edit: a structural fallback would measure nothing.
        answered += usize::from(by_checker(&answer));
        stale += usize::from(!answer.entry.props.contains_key(&format!("edit{n}")));
        if let Some(pid) = checker.process_id() {
            processes.insert(pid);
            peak = peak.max(process_stats(pid).map_or(0, |(mib, _)| mib));
        }
    }
    // A replacement follows the reply it is due after, so one more request settles the counters.
    drop(checker.request(Vec::new()).recv_timeout(Duration::from_secs(120)));
    let last = session.component("GateEdited").unwrap();
    let (fresh_session, _fresh) = open(src, tsc, Vec::new(), None);
    let fresh = fresh_session.component("GateEdited").unwrap();
    let recycles = checker.recycles();
    let ok = answered == edits && stale == 0 && last == fresh;
    println!(
        "{{\"gate\":\"memory\",\"edits\":{edits},\"maxRssMib\":{},\"answeredByChecker\":{answered},\"staleAnswers\":{stale},\"processes\":{},\"recycles\":{{\"worn\":{},\"bloated\":{}}},\"peakRssMib\":{peak},\"finalEqualsFresh\":{},\"ok\":{ok}}}",
        max_rss_mib.map_or("null".to_string(), |mib| mib.to_string()),
        processes.len(),
        recycles.worn,
        recycles.bloated,
        last == fresh,
    );
    ok
}

fn determinism(src: &Utf8PathBuf, tsc: &str, components: usize, threads: &[String], runs: usize) -> bool {
    let (probe, _checker) = open(src, tsc, Vec::new(), None);
    let mut keys = probe.component_keys();
    keys.retain(|key| key != "GateEdited");
    keys.sort();
    let step = (keys.len() / components.max(1)).max(1);
    let keys: Vec<String> = keys.into_iter().step_by(step).take(components).collect();
    drop(probe);
    let mut reference: Option<String> = None;
    let (mut ok, mut variants) = (true, Vec::new());
    for count in threads {
        for run in 0..runs {
            let env = vec![("RAYON_NUM_THREADS".to_string(), count.clone())];
            let (session, checker) = open(src, tsc, env, None);
            let mut order = keys.clone();
            if run % 2 == 1 {
                order.reverse();
            }
            let (mut text, mut structural, mut most_threads) = (Vec::new(), Vec::new(), 0);
            for key in &order {
                let answer = session.component(key).expect("a sampled key is a component");
                if !by_checker(&answer) {
                    structural.push(key.clone());
                }
                text.push((
                    key.clone(),
                    format!("{}\n{:?}", serde_json::to_string(&answer.entry).unwrap(), answer.diagnostics),
                ));
                if let Some((_, count)) = checker.process_id().and_then(process_stats) {
                    most_threads = most_threads.max(count);
                }
            }
            text.sort();
            let bytes = text.iter().map(|(key, body)| format!("{key}\n{body}\n")).collect::<String>();
            let same = reference.get_or_insert_with(|| bytes.clone()) == &bytes;
            // A component the checker declines keeps its structural answer; that is deterministic too, and is listed.
            ok &= same;
            structural.sort();
            variants.push(format!(
                "{{\"threads\":{count},\"run\":{run},\"reversed\":{},\"bytes\":{},\"answeredByChecker\":{},\"structuralOnly\":{structural:?},\"maxProcessThreads\":{most_threads},\"equalsFirst\":{same}}}",
                run % 2 == 1,
                bytes.len(),
                order.len() - structural.len()
            ));
        }
    }
    println!(
        "{{\"gate\":\"determinism\",\"components\":{},\"variants\":[{}],\"ok\":{ok}}}",
        keys.len(),
        variants.join(",")
    );
    ok
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = Utf8PathBuf::from(arg(&args, "--src").expect("--src"));
    let src = Utf8PathBuf::from_path_buf(std::fs::canonicalize(src).unwrap()).unwrap();
    let tsc = arg(&args, "--tsc").expect("--tsc");
    let number = |name: &str, default: usize| arg(&args, name).map_or(default, |n| n.parse().expect("a number"));
    let ok = match args.get(1).map(String::as_str) {
        Some("memory") => memory(
            &src,
            &tsc,
            number("--edits", 650),
            arg(&args, "--max-rss-mib").map(|n| n.parse().expect("a number")),
        ),
        Some("determinism") => {
            let threads = arg(&args, "--threads").unwrap_or_else(|| "1,2,4".to_string());
            determinism(
                &src,
                &tsc,
                number("--components", 150),
                &threads.split(',').map(str::to_string).collect::<Vec<_>>(),
                number("--runs", 3),
            )
        }
        _ => {
            eprintln!("usage: gate_harness memory|determinism --src DIR --tsc PATH");
            false
        }
    };
    std::process::exit(i32::from(!ok));
}
