//! Where a declaration comes from, read from its file's path: the one place both lanes decide it (ADR 0028, Ambient).
//!
//! Relevance reads [`is_ambient_declaration`], the checker merge reads [`Origin::React`] (what `@types/react` declares
//! it leaves to the inheritance layers) and the dependency loader leaves [`Origin::React`] files to the resolver's
//! React handling. Each reads its own question of the same answer, so a path cannot be ambient to one and plain to
//! another.
//!
//! Ambient is decided per declaration, not per file: React's own types are ambient by provenance, and a library's
//! declaration only when its typings say it is not the library's API ([`marked_private`]). React Aria's
//! `@react-types/shared/src/dom.d.ts` declares both kinds: `GlobalDOMAttributes` and `GlobalDOMEvents` (`@private`
//! copies of React's DOM attributes and events) and its documented API (`DOMProps`'s `id`, `AriaLabelingProps`,
//! `FocusableDOMProps`'s `excludeFromTabOrder`, `LinkDOMProps`'s `routerOptions`).

use std::collections::BTreeMap;

/// Where a declaration's file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// React's own types: `@types/react`, or the `react` package.
    React,
    /// The DOM types react-aria shares across its packages: `@react-types/shared/src/dom.d.ts`. Not ambient as a file:
    /// it declares API too. Only the resolver's collapse threshold reads it (`crate::types::ambient`).
    SharedDom,
    /// Any other file under `node_modules`.
    Dependency,
    /// The project's own files.
    Project,
}

impl Origin {
    /// Whether everything the file declares is ambient: what every element has, decided by provenance (React's own
    /// types), never by name. A library's file is not ambient as a whole: see [`is_ambient_declaration`].
    ///
    /// ```
    /// use caliper_docgen_core::origin::{Origin, declaration_origin};
    ///
    /// assert!(declaration_origin("/p/node_modules/@types/react/index.d.ts").is_ambient());
    /// assert!(!declaration_origin("/p/node_modules/@react-types/shared/src/dom.d.ts").is_ambient());
    /// assert_eq!(declaration_origin("/p/node_modules/@mantine/core/lib/index.d.ts"), Origin::Dependency);
    /// assert!(!declaration_origin("/p/src/Button.tsx").is_ambient());
    /// ```
    pub fn is_ambient(self) -> bool {
        match self {
            Self::React => true,
            Self::SharedDom | Self::Dependency | Self::Project => false,
        }
    }
}

/// Whether a declaration's own JSDoc says it is not its library's API: a `@private` or `@internal` modifier tag, the
/// library's statement, read from its typings and never from a name. React Aria marks its copies of React's DOM
/// attributes and events so (`GlobalDOMAttributes`, `GlobalDOMEvents`), and none of its documented API.
pub fn marked_private(tags: &BTreeMap<String, String>) -> bool {
    tags.contains_key("private") || tags.contains_key("internal")
}

/// Whether a declaration in `file` whose JSDoc tags are `tags` is ambient: React's own (by provenance), or one marked
/// private ([`marked_private`]) in a library's typings: a dependency's file or a declaration file (`.d.ts`), as React
/// Aria's `dom.d.ts` is in its own repository. A project's own source file is never ambient: an `@internal` base of the
/// project's props is still its API. `tags` is `None` when the declaration was not found: not proven, so not ambient.
pub fn is_ambient_declaration(file: &str, tags: Option<&BTreeMap<String, String>>) -> bool {
    let origin = declaration_origin(file);
    let typings = origin != Origin::Project || [".d.ts", ".d.mts", ".d.cts"].iter().any(|ext| file.ends_with(ext));
    origin.is_ambient() || (typings && tags.is_some_and(marked_private))
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
