//! The heap budget of a whole-project run ([`crate::resolve`] and the calls beside it, which `extract` makes).
//!
//! A converted type is a tree the run builds in its own heap, and a type that expands to millions of members (a style
//! system declared in the project's own source, with hundreds of properties each over a hundred conditions) builds
//! tens of GiB of them, most of it copies of trees the converter recalls from its memo between two requests. The budget
//! is the safety net under every such shape: the run reads the heap before each component and every [`STEPS_PER_READ`]
//! steps (a checker request, or a type the converter converts or recalls), and once the heap is past the budget it
//! takes no further step: the conversion in flight stops at its next one. Every component it has not answered by then, the one in flight included, keeps its
//! structural result, and one warning says how many and why. Which components those are depends on how far each lane
//! had got, so past the budget the output depends on timing; a run that stays under it is unchanged.
//!
//! The heap is what mimalloc, the allocator of the CLI and of the native binding, has committed
//! (`mi_process_info`): the converter's trees, its memo and the merged props, all the Rust memory of the run. It
//! leaves out the checker's arenas, a range of their own that [`OnDemand`](crate::OnDemand)'s bound reads. A process
//! whose allocator is not mimalloc reads only mimalloc's own small commit, and so never reaches the budget.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use caliper_docgen_core::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

use crate::rpc::CallError;

/// The most a budget derived from the machine allows: 8 GiB, about seven times the 1.1 GiB the largest corpus measured
/// commits (Mantine's core, shadcn, `gate-large`; docs/type-checker-integration.md).
pub const MAX_HEAP_BUDGET: u64 = 8 << 30;

/// How many steps (checker requests, and types converted or recalled from the memo) a run takes between two reads of
/// the heap, besides the read before each component. A read costs about 0.9 µs (`mi_process_info`, measured over
/// 100,000 reads on an M-series Mac), so a read every 32 steps is about 30 ns a step.
const STEPS_PER_READ: u64 = 32;

/// How large a whole-project run's heap may grow before it stops asking the checker (the module's documentation).
#[derive(Debug, Clone, Copy)]
pub struct HeapBudget {
    bytes: u64,
    meter: fn() -> u64,
}

impl HeapBudget {
    /// The budget of this machine: 60% of the memory this process may use, at most [`MAX_HEAP_BUDGET`] (that, where
    /// the memory cannot be read). The memory is the physical memory, or on Linux the memory limit of the process's
    /// cgroup (a container's, a CI job's) when that is lower. 60%, because the heap is not all the process holds (the
    /// checker's arenas, the structural result, the JSON being written, and in the binding the host's own heap), and a
    /// run past that memory swaps or is killed rather than finishing.
    pub fn machine() -> Self {
        let usable = match (physical_memory(), cgroup_limit()) {
            (Some(physical), Some(limit)) => Some(physical.min(limit)),
            (physical, limit) => physical.or(limit),
        };
        Self::new(usable.map_or(MAX_HEAP_BUDGET, |bytes| (bytes / 10 * 6).min(MAX_HEAP_BUDGET)))
    }

    /// A budget of `bytes` of mimalloc's committed heap.
    pub fn new(bytes: u64) -> Self {
        Self { bytes, meter: committed }
    }

    /// A budget of `bytes` as `meter` reads the heap: tests stand in a meter whose readings they choose.
    #[cfg(test)]
    pub(crate) fn metered(bytes: u64, meter: fn() -> u64) -> Self {
        Self { bytes, meter }
    }

    /// The bytes the budget allows.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// The bytes mimalloc has committed for the whole process, as it counts them.
pub(crate) fn committed() -> u64 {
    let mut commit = 0usize;
    let null = std::ptr::null_mut();
    // SAFETY: `mi_process_info` writes each out-parameter that is not null and reads nothing else; `commit` outlives
    // the call.
    #[allow(unsafe_code)]
    unsafe {
        libmimalloc_sys::mi_process_info(null, null, null, null, null, &raw mut commit, null, null);
    }
    commit as u64
}

/// The machine's physical memory in bytes.
#[cfg(unix)]
fn physical_memory() -> Option<u64> {
    // SAFETY: `sysconf` reads a system constant and has no other effect.
    #[allow(unsafe_code)]
    let (pages, size) = unsafe { (libc::sysconf(libc::_SC_PHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
    u64::try_from(pages).ok()?.checked_mul(u64::try_from(size).ok()?)
}

/// Elsewhere the machine's memory is not read.
#[cfg(not(unix))]
fn physical_memory() -> Option<u64> {
    None
}

/// The lowest memory limit of the cgroups this process is in, from its own (`/proc/self/cgroup`) up to the root of the
/// hierarchy's mount (`/proc/self/mountinfo`), in bytes. None when no limit is set, and equally when nothing can be
/// found: the files unreadable, no cgroup mount listed, or the process's cgroup outside what the mount shows. The
/// caller then falls back to the host's physical memory, so a limit that cannot be read is not enforced.
#[cfg(target_os = "linux")]
fn cgroup_limit() -> Option<u64> {
    let own = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    limit_files(&own, &mounts)
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .filter_map(|text| parse_limit(&text))
        .min()
}

/// Only Linux has cgroups.
#[cfg(not(target_os = "linux"))]
fn cgroup_limit() -> Option<u64> {
    None
}

/// A cgroup hierarchy's mount: where it is mounted, and which of the hierarchy's directories is the mount's root
/// (`/` unless the mount is a bind mount of a subtree, as in a container).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct CgroupMount<'a> {
    point: String,
    root: &'a str,
}

/// The mount of the cgroup v1 hierarchy with the `memory` controller (its super options list it, whether it is mounted
/// alone or beside `cpu,cpuacct`) and of the cgroup v2 hierarchy, from `/proc/self/mountinfo`. A line that is not
/// well formed is skipped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn cgroup_mounts(mountinfo: &str) -> (Option<CgroupMount<'_>>, Option<CgroupMount<'_>>) {
    let (mut v1, mut v2) = (None, None);
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // The optional fields end at the `-`, which cannot come before the sixth field.
        let Some(dash) = fields.iter().skip(5).position(|field| *field == "-").map(|at| at + 5) else { continue };
        let (Some(root), Some(point), Some(kind)) = (fields.get(3), fields.get(4), fields.get(dash + 1)) else {
            continue;
        };
        let mount = || CgroupMount { point: unescape(point), root };
        match *kind {
            "cgroup2" if v2.is_none() => v2 = Some(mount()),
            "cgroup" if v1.is_none() => {
                let options = fields.get(dash + 3).copied().unwrap_or_default();
                if options.split(',').any(|option| option == "memory") {
                    v1 = Some(mount());
                }
            }
            _ => {}
        }
    }
    (v1, v2)
}

/// A mountinfo path with its octal escapes (`\040` for a space) decoded.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn unescape(path: &str) -> String {
    let (bytes, mut out, mut at) = (path.as_bytes(), Vec::with_capacity(path.len()), 0);
    while at < bytes.len() {
        let octal = bytes.get(at + 1..at + 4).filter(|_| bytes[at] == b'\\');
        match octal.and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok()) {
            Some(byte) => {
                out.push(byte);
                at += 4;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The files that hold the memory limits of the cgroups `/proc/self/cgroup` (`own`) lists, each cgroup's and every
/// ancestor's up to the root of its mount: cgroup v2's `memory.max` (the `0::` line) and cgroup v1's
/// `memory.limit_in_bytes` (the line whose controllers include `memory`), each under the mount point `mountinfo` gives
/// for that hierarchy ([`cgroup_mounts`]). The process's cgroup path is relative to the hierarchy's root, so a mount
/// whose root is a subtree shows only the cgroups under it, and a cgroup outside it has no files. A hierarchy with no
/// mount has none.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn limit_files(own: &str, mountinfo: &str) -> Vec<String> {
    let (v1, v2) = cgroup_mounts(mountinfo);
    let mut files = Vec::new();
    for line in own.lines() {
        let mut fields = line.splitn(3, ':');
        let (Some(_), Some(controllers), Some(path)) = (fields.next(), fields.next(), fields.next()) else { continue };
        let (mount, file) = match controllers {
            "" => (&v2, "memory.max"),
            _ if controllers.split(',').any(|controller| controller == "memory") => (&v1, "memory.limit_in_bytes"),
            _ => continue,
        };
        let Some(mount) = mount else { continue };
        let root = mount.root.trim_end_matches('/');
        let path = path.trim_end_matches('/');
        let Some(mut dir) = path.strip_prefix(root).filter(|rest| rest.is_empty() || rest.starts_with('/')) else {
            continue;
        };
        let point = mount.point.trim_end_matches('/');
        loop {
            files.push(format!("{point}{dir}/{file}"));
            match dir.rfind('/') {
                Some(parent) => dir = &dir[..parent],
                None => break,
            }
        }
    }
    files
}

/// A cgroup memory limit file's bytes; none for no limit: `max` (v2), or a value past 2^60 (v1 writes about 2^63 for
/// no limit).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_limit(text: &str) -> Option<u64> {
    text.trim().parse::<u64>().ok().filter(|bytes| *bytes < 1 << 60)
}

tokio::task_local! {
    /// The watch of the run the current task answers for ([`step`]). A task outside a run's lanes has none, and is never
    /// stopped.
    pub(crate) static HEAP: std::sync::Arc<Watch>;
}

/// One step of the run the current task answers for: a checker request, or a type converted or recalled. Refused once
/// the run's heap is past its budget.
pub(crate) fn step() -> Result<(), CallError> {
    match HEAP.try_with(|watch| watch.allows_step()) {
        Ok(false) => Err(CallError::OverBudget),
        Ok(true) | Err(_) => Ok(()),
    }
}

/// One run's reading of its heap against its budget, shared by its lanes. Once past, it stays past.
pub(crate) struct Watch {
    budget: Option<HeapBudget>,
    steps: AtomicU64,
    peak: AtomicU64,
    /// The heap the read that found it past the budget saw.
    held: OnceLock<u64>,
}

impl Watch {
    /// A watch over `budget`; with none, the run is never stopped and the heap never read.
    pub(crate) fn new(budget: Option<HeapBudget>) -> Self {
        Self { budget, steps: AtomicU64::new(0), peak: AtomicU64::new(0), held: OnceLock::new() }
    }

    /// Reads the heap; whether it is past the budget, now or before.
    pub(crate) fn past(&self) -> bool {
        let Some(budget) = self.budget else { return false };
        if self.held.get().is_some() {
            return true;
        }
        let held = (budget.meter)();
        self.peak.fetch_max(held, Ordering::Relaxed);
        if held > budget.bytes {
            self.held.get_or_init(|| held);
            return true;
        }
        false
    }

    /// Whether the heap was found past the budget, without reading it.
    pub(crate) fn passed(&self) -> bool {
        self.held.get().is_some()
    }

    /// Whether the run may take one more step: not once the heap is past the budget, which every [`STEPS_PER_READ`]th
    /// step reads.
    fn allows_step(&self) -> bool {
        if self.budget.is_none() {
            return true;
        }
        if self.passed() {
            return false;
        }
        let taken = self.steps.fetch_add(1, Ordering::Relaxed);
        !(taken.is_multiple_of(STEPS_PER_READ) && self.past())
    }

    /// The most the heap held at a read, in bytes; 0 when it was never read.
    pub(crate) fn peak(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }

    /// The one warning for the components `left` structural once the heap was past the budget; none when it never
    /// was, or nothing was left.
    pub(crate) fn diagnostic(&self, mut left: Vec<String>) -> Option<Diagnostic> {
        let (Some(budget), Some(held)) = (self.budget, self.held.get()) else { return None };
        if left.is_empty() {
            return None;
        }
        left.sort();
        let mut diagnostic = Diagnostic::new(
            DiagnosticSeverity::Warning,
            DiagnosticCode::SemanticBackend,
            format!(
                "{} component(s) kept their structural result: the heap held {} MiB, past this run's {} MiB budget, \
                 so the TypeScript checker was asked nothing more. Which components the checker had answered by then \
                 depends on timing, so this output can differ from run to run",
                left.len(),
                held >> 20,
                budget.bytes >> 20
            ),
        )
        .with_help(
            "The budget is 60% of the memory this process may use (the machine's, or its cgroup limit on Linux), at most 8 GiB. A type that expands to millions of members, \
             such as a style system declared in the project's own source, is the usual cause; `--no-checker` gives \
             the structural result for every component.",
        );
        diagnostic.affects = left;
        Some(diagnostic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2: &str = "29 22 0:26 / /sys/fs/cgroup rw,nosuid,nodev,noexec,relatime - cgroup2 cgroup2 rw,nsdelegate\n";
    const V1_COMOUNTED: &str = "30 23 0:27 / /sys/fs/cgroup/cpu,cpuacct,memory rw,nosuid,relatime shared:12 - cgroup cgroup rw,cpu,cpuacct,memory\n";
    const V1_SEPARATE: &str = "31 23 0:28 / /sys/fs/cgroup/cpu,cpuacct rw,relatime - cgroup cgroup rw,cpu,cpuacct\n\
                               32 23 0:29 / /sys/fs/cgroup/memory rw,relatime - cgroup cgroup rw,memory\n";

    /// A cgroup v2 process (a container, a systemd unit) reads `memory.max` in its own cgroup and every ancestor's.
    #[test]
    fn a_v2_cgroup_reads_memory_max_up_to_the_root() {
        assert_eq!(
            limit_files("0::/system.slice/docker-1.scope\n", V2),
            [
                "/sys/fs/cgroup/system.slice/docker-1.scope/memory.max",
                "/sys/fs/cgroup/system.slice/memory.max",
                "/sys/fs/cgroup/memory.max",
            ]
        );
        // Inside a container's own namespace the cgroup is the root.
        assert_eq!(limit_files("0::/\n", V2), ["/sys/fs/cgroup/memory.max"]);
    }

    /// A v1 memory controller comounted with others is found at that mount, not at `/sys/fs/cgroup/memory`.
    #[test]
    fn a_comounted_v1_memory_controller_is_found_at_its_mount() {
        let own = "12:cpu,cpuacct,memory:/docker/abc\n1:name=systemd:/docker/abc\n";
        assert_eq!(
            limit_files(own, V1_COMOUNTED),
            [
                "/sys/fs/cgroup/cpu,cpuacct,memory/docker/abc/memory.limit_in_bytes",
                "/sys/fs/cgroup/cpu,cpuacct,memory/docker/memory.limit_in_bytes",
                "/sys/fs/cgroup/cpu,cpuacct,memory/memory.limit_in_bytes",
            ]
        );
    }

    /// A v1 process reads the `memory` hierarchy only, from the mount whose options list `memory`.
    #[test]
    fn a_v1_cgroup_reads_the_memory_hierarchy_only() {
        let own = "12:cpu,cpuacct:/docker/abc\n11:memory:/docker/abc\n1:name=systemd:/docker/abc\n";
        assert_eq!(
            limit_files(own, V1_SEPARATE),
            [
                "/sys/fs/cgroup/memory/docker/abc/memory.limit_in_bytes",
                "/sys/fs/cgroup/memory/docker/memory.limit_in_bytes",
                "/sys/fs/cgroup/memory/memory.limit_in_bytes",
            ]
        );
        assert!(limit_files("garbage\n\n", V1_SEPARATE).is_empty());
    }

    /// With no cgroup mount, or none for the process's hierarchy, there is nothing to read.
    #[test]
    fn no_cgroup_mount_gives_no_files() {
        let proc = "22 1 8:1 / / rw - ext4 /dev/sda1 rw\n";
        assert!(limit_files("0::/a\n", proc).is_empty());
        assert!(limit_files("11:memory:/a\n", proc).is_empty());
        assert!(limit_files("11:memory:/a\n", V2).is_empty());
        assert!(limit_files("0::/a\n", V1_SEPARATE).is_empty());
    }

    /// A bind-mounted subtree shows the cgroups under its root: the process's path is taken relative to the root, and a
    /// cgroup outside it has no files.
    #[test]
    fn a_bind_mounted_root_is_stripped_from_the_cgroup_path() {
        let mounts = "40 30 0:26 /docker/abc /sys/fs/cgroup rw - cgroup2 cgroup2 rw\n";
        assert_eq!(
            limit_files("0::/docker/abc/sub\n", mounts),
            ["/sys/fs/cgroup/sub/memory.max", "/sys/fs/cgroup/memory.max"]
        );
        assert_eq!(limit_files("0::/docker/abc\n", mounts), ["/sys/fs/cgroup/memory.max"]);
        assert!(limit_files("0::/docker/abcd\n", mounts).is_empty());
        assert!(limit_files("0::/other\n", mounts).is_empty());
    }

    /// Malformed lines are skipped without a panic, and a mount point's octal escapes are decoded.
    #[test]
    fn malformed_mountinfo_is_skipped() {
        for junk in ["", "-", "1 2 3 - cgroup", "1 2 3 / - cgroup2", "1 2 3 / /x - cgroup", "a b c d e - cgroup2"] {
            let _ = limit_files("0::/a\n11:memory:/a\n", junk);
        }
        assert!(limit_files("0::/a\n", "1 2 3 / - cgroup2 x\n").is_empty());
        let spaced = "1 2 3 / /sys/fs/cg\\040root - cgroup2 cgroup2 rw\n";
        assert_eq!(limit_files("0::/\n", spaced), ["/sys/fs/cg root/memory.max"]);
        assert_eq!(unescape("a\\04"), "a\\04");
    }

    /// A limit is a byte count; `max` and v1's near-2^63 value mean no limit, and anything unreadable is none.
    #[test]
    fn a_limit_file_gives_bytes_or_no_limit() {
        assert_eq!(parse_limit("4294967296\n"), Some(4 << 30));
        assert_eq!(parse_limit("max\n"), None);
        assert_eq!(parse_limit("9223372036854771712\n"), None);
        assert_eq!(parse_limit(""), None);
        assert_eq!(parse_limit("-1"), None);
    }
}
