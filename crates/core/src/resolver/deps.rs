//! What a component's resolution depended on, so watch mode re-resolves exactly the components an edit can change. A
//! component resolves on one thread from start to finish, so the recorder is thread-local and scoped to one
//! `resolve_component` call; lookups record into it without every resolver function threading a collector through.

use std::cell::RefCell;

use camino::Utf8Path;
use compact_str::CompactString;
use rustc_hash::FxHashSet;

use crate::path_key::PathKey;

/// Everything outside the component's own mapping that its resolution read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dependencies {
    /// Source files whose declarations, imports or exports were read, including lookups that found nothing there.
    pub(crate) files: FxHashSet<PathKey>,
    /// Enum and const-array names looked up project-wide by bare name. A file that starts or stops declaring one of
    /// these can change which declaration the name binds to.
    pub(crate) bare_names: FxHashSet<CompactString>,
    /// Whether a module specifier was resolved. A new file can change what a specifier resolves to (or make an
    /// unresolvable one resolve); an edit or deletion cannot, beyond the resolved file already in `files`.
    pub(crate) resolves_modules: bool,
}

thread_local! {
    static RECORDER: RefCell<Option<Dependencies>> = const { RefCell::new(None) };
}

fn with_recorder(f: impl FnOnce(&mut Dependencies)) {
    RECORDER.with(|recorder| {
        if let Some(deps) = recorder.borrow_mut().as_mut() {
            f(deps);
        }
    });
}

/// Note that the current resolution read `file`. A no-op outside `recording`.
pub(super) fn record(file: &Utf8Path) {
    with_recorder(|deps| {
        if !deps.files.contains(file.as_str()) {
            deps.files.insert(PathKey::from(file));
        }
    });
}

/// Note a project-wide lookup of an enum or const array by bare name.
pub(super) fn record_bare_name(name: &str) {
    with_recorder(|deps| {
        if !deps.bare_names.contains(name) {
            deps.bare_names.insert(CompactString::from(name));
        }
    });
}

/// Note that a module specifier was resolved.
pub(super) fn record_module_resolution() {
    with_recorder(|deps| deps.resolves_modules = true);
}

impl Dependencies {
    fn absorb(&mut self, other: &Dependencies) {
        self.files.extend(other.files.iter().cloned());
        self.bare_names.extend(other.bare_names.iter().cloned());
        self.resolves_modules |= other.resolves_modules;
    }
}

/// Run `f` inside the current recording and also return what `f` alone depended on, so a reused result can replay it.
pub(super) fn capturing<T>(f: impl FnOnce() -> T) -> (T, Dependencies) {
    let outer = RECORDER.with(|recorder| recorder.borrow_mut().replace(Dependencies::default()));
    let result = f();
    let inner = RECORDER.with(|recorder| recorder.borrow_mut().take()).unwrap_or_default();
    RECORDER.with(|recorder| {
        *recorder.borrow_mut() = outer.map(|mut deps| {
            deps.absorb(&inner);
            deps
        });
    });
    (result, inner)
}

/// Record `deps` into the current recording, as if the lookups behind them had run again.
pub(super) fn replay(deps: &Dependencies) {
    with_recorder(|current| current.absorb(deps));
}

/// Run `f` and return what it depended on. Not reentrant: resolution never nests a component inside another.
pub(super) fn recording<T>(f: impl FnOnce() -> T) -> (T, Dependencies) {
    RECORDER.with(|recorder| *recorder.borrow_mut() = Some(Dependencies::default()));
    let result = f();
    let deps = RECORDER.with(|recorder| recorder.borrow_mut().take()).unwrap_or_default();
    (result, deps)
}
