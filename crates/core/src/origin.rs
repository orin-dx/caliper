//! Where a declaration comes from, read from its file's path: the one place both lanes decide it (ADR 0028, Ambient).
//!
//! Relevance reads [`Origin::is_ambient`], the checker merge reads [`Origin::React`] (what `@types/react` declares it
//! leaves to the inheritance layers) and the dependency loader leaves [`Origin::React`] files to the resolver's React
//! handling. Each reads its own question of the same answer, so a path cannot be ambient to one and plain to another.

/// Where a declaration's file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// React's own types: `@types/react`, or the `react` package.
    React,
    /// The DOM types react-aria shares across its packages: `@react-types/shared/src/dom.d.ts`.
    SharedDom,
    /// Any other file under `node_modules`.
    Dependency,
    /// The project's own files.
    Project,
}

impl Origin {
    /// Whether what the file declares is ambient: what every element has, decided by path, never by name.
    ///
    /// ```
    /// use caliper_docgen_core::origin::{Origin, declaration_origin};
    ///
    /// assert!(declaration_origin("/p/node_modules/@types/react/index.d.ts").is_ambient());
    /// assert!(declaration_origin("/p/node_modules/@react-types/shared/src/dom.d.ts").is_ambient());
    /// assert_eq!(declaration_origin("/p/node_modules/@mantine/core/lib/index.d.ts"), Origin::Dependency);
    /// assert!(!declaration_origin("/p/src/Button.tsx").is_ambient());
    /// ```
    pub fn is_ambient(self) -> bool {
        match self {
            Self::React | Self::SharedDom => true,
            Self::Dependency | Self::Project => false,
        }
    }
}

/// The [`Origin`] of a declaration in `file`, an absolute path with `/` separators.
pub fn declaration_origin(file: &str) -> Origin {
    if file.contains("/@types/react/") || file.contains("/node_modules/react/") {
        Origin::React
    } else if file
        .rsplit_once("/@react-types/shared/src/")
        .is_some_and(|(_, name)| matches!(name, "dom.d.ts" | "dom.d.mts" | "dom.d.cts"))
    {
        Origin::SharedDom
    } else if file.contains("/node_modules/") {
        Origin::Dependency
    } else {
        Origin::Project
    }
}
