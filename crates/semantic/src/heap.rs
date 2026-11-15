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
    /// The budget of this machine: half its physical memory, at most [`MAX_HEAP_BUDGET`] (that, where the memory
    /// cannot be read). Half, because the heap is not all the process holds (the checker's arenas, the structural
    /// result, the JSON being written, and in the binding the host's own heap), and a run past the machine's memory
    /// swaps until it is killed rather than finishing.
    pub fn machine() -> Self {
        Self::new(physical_memory().map_or(MAX_HEAP_BUDGET, |bytes| (bytes / 2).min(MAX_HEAP_BUDGET)))
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
            "The budget is half the machine's memory, at most 8 GiB. A type that expands to millions of members, \
             such as a style system declared in the project's own source, is the usual cause; `--no-checker` gives \
             the structural result for every component.",
        );
        diagnostic.affects = left;
        Some(diagnostic)
    }
}
