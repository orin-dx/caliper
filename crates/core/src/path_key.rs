//! A file path as a hash-map key, hashed and compared as its string.
//!
//! `Path`'s own `Hash` and `Eq` walk the path component by component, which made path-keyed lookups the resolver's
//! largest cost after module resolution. Every path the resolver keys on is canonical (discovery canonicalizes them),
//! so string equality is path equality.

use std::borrow::Borrow;

use camino::Utf8Path;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PathKey(Box<str>);

impl From<&Utf8Path> for PathKey {
    fn from(path: &Utf8Path) -> PathKey {
        PathKey(Box::from(path.as_str()))
    }
}

/// Lets a `HashMap<PathKey, _>` be queried with `path.as_str()`, without allocating a key.
impl Borrow<str> for PathKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}
