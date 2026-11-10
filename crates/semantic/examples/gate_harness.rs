#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, clippy::print_stderr)]
//! The measurable ADR 0026 gates, through the session a Storybook addon runs: a lazy `WatchSession` asking a warm
//! `OnDemand` over the project's own `tsc` (`just gate-measure` runs all four).
//!
//!   gate_harness memory       --src DIR --tsc PATH [--edits 650] [--max-rss-mib N]
//!   gate_harness determinism  --src DIR --tsc PATH [--components 150] [--threads 1,2,4] [--runs 3]
//!   gate_harness first-answer --src DIR --tsc PATH [--component ChakraButton] [--runs 5] [--budget-ms 150]
//!   gate_harness watch        --src DIR --tsc PATH [--edits 300] [--sample 100] [--budget-ms 25] [--max-rss-mib N]
//!                             [--props button|own]
//!
//! `first-answer` (S4) times the first answer for one component in fresh sessions: see [`first_answer`]. `watch` (S6)
//! times edit-and-query on the snapshot the checker keeps: see [`watch`].
//! `memory` rewrites one component's file, tells the session, and asks for the component, `--edits` times; it samples the
//! checker's PID and resident memory after every answer and prints the recycles, the peak, and whether the last answer
//! equals a fresh session's. `determinism` asks a fresh checker for the same components under each `RAYON_NUM_THREADS`
//! (tsrs's `--api` has no checker-count flag; see docs/gate-measurements.md), `--runs` times each, alternating the query
//! order, and byte-compares every output. Each prints one JSON line and exits 1 on a failed check.
use std::collections::BTreeSet;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use caliper_docgen_core::pipeline::{CollapseInheritedProps, ComponentAnswer, PipelineOptions, WatchSession};
use caliper_docgen_core::react_types::react_props_wanted;
use caliper_docgen_core::types::DiagnosticSeverity;
use caliper_docgen_core::types::semantic::{SemanticProvider, SemanticTarget};
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

/// CPU time a process has used, in milliseconds, from `ps` (`[hh:]mm:ss.ss`); `None` once it has gone. Unlike wall
/// time, it does not grow with the machine's load.
fn cpu_ms(pid: u32) -> Option<f64> {
    let out = Command::new("ps").args(["-o", "cputime=", "-p", &pid.to_string()]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let seconds = text.trim().split(':').try_fold(0.0, |total, part| Some(total * 60.0 + part.parse::<f64>().ok()?));
    out.status.success().then_some(seconds?).map(|seconds| seconds * 1000.0)
}

fn edit_source(n: usize) -> String {
    format!(
        "import type {{ ButtonProps }} from '@chakra-ui/react'\nexport function GateEdited(props: ButtonProps & {{ edit{n}: string }}) {{\n  return null\n}}\n"
    )
}

/// The same program (the file still imports Chakra, so the checker loads it whole), but a component with one prop of
/// its own: what an edit costs the checker, with almost nothing to convert.
fn own_props_source(n: usize) -> String {
    format!(
        "import type {{ ButtonProps }} from '@chakra-ui/react'\nexport type Kept = ButtonProps\nexport function GateEdited(props: {{ edit{n}: string }}) {{\n  return null\n}}\n"
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

fn percentile(sorted: &[f64], at: f64) -> f64 {
    let index = ((sorted.len() as f64 * at).ceil() as usize).clamp(1, sorted.len().max(1)) - 1;
    sorted.get(index).copied().unwrap_or(0.0)
}

/// Each answer's JSON entry and diagnostics, in `keys` order.
fn answer_bytes(session: &WatchSession, keys: &[String]) -> Vec<String> {
    keys.iter()
        .map(|key| {
            let answer = session.component(key).expect("a sampled key is a component");
            format!("{key}\n{}\n{:?}", serde_json::to_string(&answer.entry).unwrap(), answer.diagnostics)
        })
        .collect()
}

/// The options of [`watch`].
struct WatchRun {
    edits: usize,
    sample: usize,
    budget_ms: f64,
    max_rss_mib: Option<u64>,
    own_props: bool,
}

/// ADR 0026 S6: edit-and-query on the kept snapshot. Rewrites one component's file (`ButtonProps & { edit{n}: string }`),
/// applies it with `update_file`, which tells the checker, and times that and the component's answer, `--edits` times,
/// sampling the checker's resident memory after each. Then it times the checker's answer alone on its unchanged
/// snapshot, and byte-compares the edited component and every `--sample`th other component with a fresh session asked
/// in reverse order. Passes when every answer is the checker's, carries its edit and no warning, memory stays under
/// `--max-rss-mib`, the bytes match, and the p95 is at most `--budget-ms`. `--props own` edits a component with one prop
/// of its own in the same program instead ([`own_props_source`]).
fn watch(src: &Utf8PathBuf, tsc: &str, run: WatchRun) -> bool {
    let WatchRun { edits, sample, budget_ms, max_rss_mib, own_props } = run;
    let source = if own_props { own_props_source } else { edit_source };
    let file = src.join("GateEdited.tsx");
    std::fs::write(&file, source(0)).unwrap();
    let (session, checker) = open(src, tsc, Vec::new(), max_rss_mib);
    let threshold = max_rss_mib.unwrap_or(OnDemandConfig::new(src.clone()).max_rss_mib);
    drop(session.component("GateEdited").expect("GateEdited is a component"));
    let (mut times, mut updates, mut processes, mut peak) = (Vec::new(), Vec::new(), BTreeSet::new(), 0);
    let (mut answered, mut stale, mut warned) = (0, 0, 0);
    // CPU time is what an edit costs whatever else the machine runs: the checker's, each process's last reading.
    let (own_cpu, first) = (cpu_ms(std::process::id()), checker.process_id());
    let first_cpu = first.and_then(cpu_ms).unwrap_or(0.0);
    let mut checker_cpu = std::collections::BTreeMap::new();
    for n in 1..=edits {
        std::fs::write(&file, source(n)).unwrap();
        let began = Instant::now();
        session.update_file(&file);
        updates.push(began.elapsed().as_secs_f64() * 1000.0);
        let answer = session.component("GateEdited").expect("GateEdited is a component");
        times.push(began.elapsed().as_secs_f64() * 1000.0);
        answered += usize::from(by_checker(&answer));
        stale += usize::from(!answer.entry.props.contains_key(&format!("edit{n}")));
        warned += usize::from(answer.diagnostics.iter().any(|d| d.severity == DiagnosticSeverity::Warning));
        if let Some(pid) = checker.process_id() {
            processes.insert(pid);
            peak = peak.max(process_stats(pid).map_or(0, |(mib, _)| mib));
            if let Some(ms) = cpu_ms(pid) {
                checker_cpu.insert(pid, ms);
            }
        }
    }
    let checker_cpu = (checker_cpu.values().sum::<f64>() - first_cpu) / edits as f64;
    let own_cpu = own_cpu.zip(cpu_ms(std::process::id())).map_or(0.0, |(start, end)| (end - start) / edits as f64);
    let snapshots = checker.snapshots();

    // The checker's answer alone: the same request the session sends, on the snapshot it keeps, nothing changed.
    let entry = session.component("GateEdited").unwrap().entry;
    let target = SemanticTarget {
        key: "GateEdited".to_string(),
        file_path: entry.file_path.clone(),
        display_name: entry.display_name.clone(),
        exports: entry.exports.clone(),
        described: entry.props.values().filter(|p| !p.description.is_empty()).map(|p| p.name.clone()).collect(),
        react_wanted: react_props_wanted(session.options.html_attributes, &entry.props),
    };
    let mut query = Vec::new();
    for _ in 0..21 {
        let began = Instant::now();
        drop(checker.request(vec![target.clone()]).recv_timeout(Duration::from_secs(120)));
        query.push(began.elapsed().as_secs_f64() * 1000.0);
    }

    let mut keys = session.component_keys();
    keys.retain(|key| key != "GateEdited");
    keys.sort();
    let mut keys: Vec<String> = keys.into_iter().step_by(sample.max(1)).collect();
    keys.insert(0, "GateEdited".to_string());
    let watched = answer_bytes(&session, &keys);
    let (fresh_session, _fresh) = open(src, tsc, Vec::new(), None);
    let reversed: Vec<String> = keys.iter().rev().cloned().collect();
    let mut fresh = answer_bytes(&fresh_session, &reversed);
    fresh.reverse();
    let identical = watched == fresh;
    let recycles = checker.recycles();

    times.sort_by(f64::total_cmp);
    updates.sort_by(f64::total_cmp);
    query.sort_by(f64::total_cmp);
    let (p50, p95, p99) = (percentile(&times, 0.5), percentile(&times, 0.95), percentile(&times, 0.99));
    let ok = answered == edits && stale == 0 && warned == 0 && identical && peak <= threshold && p95 <= budget_ms;
    println!(
        "{{\"gate\":\"watch\",\"props\":\"{}\",\"edits\":{edits},\"answeredByChecker\":{answered},\"staleAnswers\":{stale},\"warnedAnswers\":{warned},\"snapshots\":{{\"created\":{},\"updated\":{}}},\"processes\":{},\"recycles\":{{\"worn\":{},\"bloated\":{}}},\"peakRssMib\":{peak},\"maxRssMib\":{threshold},\"editAndQueryMs\":{{\"p50\":{p50:.1},\"p95\":{p95:.1},\"p99\":{p99:.1},\"max\":{:.1}}},\"updateFileMs\":{{\"p50\":{:.1}}},\"cpuMsPerEdit\":{{\"checker\":{checker_cpu:.1},\"caliper\":{own_cpu:.1}}},\"queryOnlyMs\":{{\"p50\":{:.1},\"min\":{:.1}}},\"budgetMs\":{budget_ms},\"comparedComponents\":{},\"comparedBytes\":{},\"equalsFreshReversed\":{identical},\"ok\":{ok}}}",
        if own_props { "own" } else { "button" },
        snapshots.created,
        snapshots.updated,
        processes.len(),
        recycles.worn,
        recycles.bloated,
        times.last().copied().unwrap_or(0.0),
        percentile(&updates, 0.5),
        percentile(&query, 0.5),
        query.first().copied().unwrap_or(0.0),
        keys.len(),
        watched.iter().map(String::len).sum::<usize>(),
    );
    ok
}

/// ADR 0026 S4: how long a story waits for the checker's answer through the Rust client. Each run opens a fresh session
/// as the addon does (the checker started in the background at creation), waits until that process is up, then asks for
/// `key` once and times the whole answer: the structural pass and the checker's program and conversion for the
/// component. Passes when every answer is the checker's and the median is at most `budget_ms`, the addon's default
/// `semanticWaitMs`.
fn first_answer(src: &Utf8PathBuf, tsc: &str, key: &str, runs: usize, budget_ms: u128) -> bool {
    let (mut start, mut first, mut structural, mut answered) = (Vec::new(), Vec::new(), Vec::new(), 0);
    for _ in 0..runs.max(1) {
        let plain = WatchSession::lazy(PipelineOptions {
            src_dirs: vec![src.clone()],
            load_dependencies: true,
            collapse_inherited_props: CollapseInheritedProps::None,
            ..PipelineOptions::default()
        });
        drop(plain.initialize());
        let began = Instant::now();
        drop(plain.component(key).expect("the key is a component"));
        structural.push(began.elapsed().as_millis());

        let (session, checker) = open(src, tsc, Vec::new(), None);
        let began = Instant::now();
        checker.warm_up();
        // Queued behind the warm-up, an empty request returns once the process is up and initialized.
        drop(checker.request(Vec::new()).recv_timeout(Duration::from_secs(120)));
        start.push(began.elapsed().as_millis());
        let began = Instant::now();
        let answer = session.component(key).expect("the key is a component");
        first.push(began.elapsed().as_millis());
        answered += usize::from(by_checker(&answer));
    }
    let median = |values: &mut Vec<u128>| {
        values.sort_unstable();
        values[values.len() / 2]
    };
    let (start_ms, first_ms, structural_ms) = (median(&mut start), median(&mut first), median(&mut structural));
    let ok = answered == first.len() && first_ms <= budget_ms;
    println!(
        "{{\"gate\":\"first-answer\",\"component\":{key:?},\"runs\":{},\"answeredByChecker\":{answered},\"startMs\":{start_ms},\"structuralMs\":{structural_ms},\"firstAnswerMs\":{first_ms},\"firstAnswerEachMs\":{first:?},\"budgetMs\":{budget_ms},\"ok\":{ok}}}",
        first.len(),
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
        Some("watch") => watch(
            &src,
            &tsc,
            WatchRun {
                edits: number("--edits", 300),
                sample: number("--sample", 100),
                budget_ms: arg(&args, "--budget-ms").map_or(25.0, |n| n.parse().expect("a number")),
                max_rss_mib: arg(&args, "--max-rss-mib").map(|n| n.parse().expect("a number")),
                own_props: arg(&args, "--props").is_some_and(|props| props == "own"),
            },
        ),
        Some("first-answer") => first_answer(
            &src,
            &tsc,
            &arg(&args, "--component").unwrap_or_else(|| "ChakraButton".to_string()),
            number("--runs", 5),
            arg(&args, "--budget-ms").map_or(150, |n| n.parse().expect("a number")),
        ),
        _ => {
            eprintln!("usage: gate_harness memory|determinism|first-answer|watch --src DIR --tsc PATH");
            false
        }
    };
    std::process::exit(i32::from(!ok));
}
