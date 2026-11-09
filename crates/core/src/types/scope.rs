//! The type parameters a type expression is read under.

use std::sync::Arc;

use compact_str::CompactString;

/// Ends the name of a type parameter reference that crossed from the place that wrote it into another declaration's
/// body as a type argument. Such a name can only be that placeholder: no declaration is named with it, and no
/// parameter of the declaration it lands in can capture it.
pub const PLACEHOLDER_MARK: char = '\u{1}';

/// Ends the name of a reference that must be looked up as a declaration, never as a type parameter: a call's type
/// argument that names a type parameter of the function it calls (`make<P>` over a module `type P`) is written
/// where that parameter is not in scope.
pub const DECLARATION_MARK: char = '\u{2}';

/// `name` marked as the declaration it refers to (`DECLARATION_MARK`).
pub fn mark_declaration(name: &str) -> CompactString {
    let mut marked = CompactString::from(name);
    marked.push(DECLARATION_MARK);
    marked
}

/// Whether `name` is a placeholder reference marked by `mark_placeholder`.
pub fn is_marked_placeholder(name: &str) -> bool {
    name.ends_with(PLACEHOLDER_MARK)
}

/// `name` marked as the placeholder it refers to (`PLACEHOLDER_MARK`).
pub fn mark_placeholder(name: &str) -> CompactString {
    let mut marked = CompactString::from(name);
    marked.push(PLACEHOLDER_MARK);
    marked
}

/// The name a marked reference stands for, or `name` itself when it is not marked.
pub fn unmarked(name: &str) -> &str {
    name.strip_suffix([PLACEHOLDER_MARK, DECLARATION_MARK]).unwrap_or(name)
}

/// The type parameters in scope where a type is read: a name here is the innermost binding of that name and shadows any
/// declaration of the same name.
///
/// Declarations (interfaces, aliases, components) sit at module level, so a declaration's body is read under its own
/// parameters only, never under those of whoever refers to it. A type argument is the opposite: its references to the
/// parameters of the place that wrote it are marked when it is bound (`mark_placeholder`), so it means the same wherever
/// a declaration's body ends up using it, and nothing reads it under a scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct TypeScope(Option<Arc<[CompactString]>>);

impl TypeScope {
    /// The scope that binds `names`.
    pub fn new<'n>(names: impl IntoIterator<Item = &'n CompactString>) -> Self {
        let mut names: Vec<CompactString> = names.into_iter().cloned().collect();
        names.sort_unstable();
        names.dedup();
        if names.is_empty() { Self(None) } else { Self(Some(names.into())) }
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
