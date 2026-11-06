//! The type parameters a type expression is read under.

use std::sync::Arc;

use compact_str::CompactString;

/// The type parameters in scope where a type is read: a name here is the innermost binding of that name and shadows any
/// declaration of the same name.
///
/// Declarations (interfaces, aliases, components) sit at module level, so a declaration's body is read under its own
/// parameters only, never under those of whoever refers to it. A type argument is the opposite: it is read under the
/// parameters of the place that wrote it, wherever a declaration's body ends up using it (`CollectedType::AtFile`
/// carries that scope next to the file the argument was written in).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct TypeScope(Option<Arc<[CompactString]>>);

impl TypeScope {
    /// The scope that binds `names`.
    pub fn new<'n>(names: impl IntoIterator<Item = &'n CompactString>) -> Self {
        let mut names: Vec<CompactString> = names.into_iter().cloned().collect();
        names.sort_unstable();
        names.dedup();
        if names.is_empty() {
            Self(None)
        } else {
            Self(Some(names.into()))
        }
    }

    /// Whether `name` is a type parameter in this scope.
    pub fn contains(&self, name: &str) -> bool {
        self.0.as_deref().is_some_and(|names| names.binary_search_by(|bound| bound.as_str().cmp(name)).is_ok())
    }

    /// Whether this scope binds nothing.
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// The names this scope binds.
    pub fn names(&self) -> &[CompactString] {
        self.0.as_deref().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_binds_exactly_the_names_it_was_made_from() {
        let scope = TypeScope::new(&[CompactString::from("T"), CompactString::from("Value"), CompactString::from("T")]);
        assert_eq!(scope.names(), ["T", "Value"]);
        assert!(scope.contains("Value"));
        assert!(!scope.contains("U"));
        assert!(!TypeScope::default().contains("T"));
        assert_eq!(TypeScope::new(&Vec::<CompactString>::new()), TypeScope::default());
    }
}
