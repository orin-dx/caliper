//! The names the per-file binding pass vouches for, as types. A candidate only a peel makes readable
//! (`component::Recognition::Peeled`) relies on a bare name only through a [`UniqueName`], [`GlobalName`] or
//! [`ImportedName`], and nothing but [`Bindings`] makes one: a path that would skip the check does not compile.

use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use super::component::Recognition;

/// What the one binding pass over a file found ([`super::component::file_names`]).
#[derive(Default)]
pub(crate) struct Bindings {
    /// How many times the file declares each name, any kind in any scope.
    counts: FxHashMap<CompactString, u32>,
    /// The names an `import` declaration binds.
    imports: FxHashSet<CompactString>,
    /// The writes to a member of a name: `(object, Some(key))` for a key known statically, `(object, None)` for one
    /// through a key that is not (`Object[k] = …`), which may be any member.
    writes: FxHashSet<(CompactString, Option<CompactString>)>,
}

/// A name the file declares exactly once: the one declaration a bare use of it can mean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UniqueName(CompactString);

/// A name the file never declares: a global.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GlobalName(CompactString);

/// A name whose one declaration in the file is an import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportedName(CompactString);

impl UniqueName {
    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

impl GlobalName {
    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A name a candidate relies on: as origin/main read it (`Plain`), or read only through a peel (`Peeled`), which only
/// the binding pass can vouch for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Named {
    Plain(CompactString),
    Peeled(UniqueName),
}

impl Named {
    pub(super) fn as_str(&self) -> &str {
        match self {
            Self::Plain(name) => name,
            Self::Peeled(name) => name.as_str(),
        }
    }
}

impl Bindings {
    pub(super) fn new(
        counts: FxHashMap<CompactString, u32>,
        imports: FxHashSet<CompactString>,
        writes: FxHashSet<(CompactString, Option<CompactString>)>,
    ) -> Self {
        Self { counts, imports, writes }
    }

    /// How many times the file declares `name`.
    pub(super) fn declarations(&self, name: &str) -> u32 {
        self.counts.get(name).copied().unwrap_or(0)
    }

    /// `name`, when the file declares it exactly once.
    pub(super) fn unique(&self, name: &str) -> Option<UniqueName> {
        (self.declarations(name) == 1).then(|| UniqueName(name.into()))
    }

    /// `name`, when the file never declares it.
    pub(super) fn global(&self, name: &str) -> Option<GlobalName> {
        (self.declarations(name) == 0).then(|| GlobalName(name.into()))
    }

    /// `name`, when its one declaration is an import.
    pub(super) fn imported(&self, name: &str) -> Option<ImportedName> {
        (self.declarations(name) == 1 && self.imports.contains(name)).then(|| ImportedName(name.into()))
    }

    /// The name a candidate relies on, by how the candidate was read: `Plain` as written; `Peeled` only when unique.
    pub(super) fn named(&self, name: &str, recognition: Recognition) -> Option<Named> {
        match recognition {
            Recognition::Plain => Some(Named::Plain(name.into())),
            Recognition::Peeled => self.unique(name).map(Named::Peeled),
        }
    }

    /// Whether the file writes `member` of the global, through any key that is or may be `member`.
    pub(super) fn writes_to(&self, global: &GlobalName, member: &str) -> bool {
        let object = CompactString::from(global.as_str());
        self.writes.contains(&(object.clone(), Some(member.into()))) || self.writes.contains(&(object, None))
    }
}
