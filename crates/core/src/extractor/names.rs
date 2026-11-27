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
    /// A name as origin/main read it: written plainly, so nothing needs to vouch for it.
    pub(super) fn plain(name: &str) -> Self {
        Self::Plain(name.into())
    }

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

#[cfg(test)]
mod tests {
    /// Every `pub(super)`/`pub(crate)` function of these modules that takes a bare name (`&str`, `String` or
    /// `CompactString`) in a parameter named like a name it might look up, clone, attach or record, with the reason it may.
    /// A name a `Peeled` candidate relies on must arrive as a [`super::Named`]; a new function that takes a bare one fails
    /// here until it takes `Named` or is listed with its reason.
    const ALLOWED: &[(&str, &str, &str)] = &[
        ("component.rs", "try_record_cast_component_alias", "the declarator's own name, a declaration, not a lookup"),
        ("component.rs", "detect_component", "the declarator's own name"),
        ("component.rs", "try_fc_annotation", "the declarator's own name"),
        ("component.rs", "extract_props_from_type_annotation", "the declarator's own name"),
        ("component.rs", "try_forward_ref", "the declarator's own name"),
        ("component.rs", "try_typed_function_expression", "the declarator's own name"),
        ("component.rs", "try_hoc_wrapped", "the declarator's own name"),
        ("component.rs", "try_exotic_component_decl", "the declarator's own name"),
        ("component.rs", "try_declared_callable_component", "the declarator's own name"),
        ("component.rs", "try_class_component", "the class's own name"),
        ("component.rs", "try_class_expression_wrapped", "the declarator's own name"),
        ("component.rs", "try_rename_identifier_wrapped_component", "the declarator's own name"),
        ("component.rs", "try_defer_identifier_alias", "the declarator's own name; the aliased one is a `Named`"),
        ("component.rs", "try_record_imported_member_alias", "the declarator's own name; the object is a `Named`"),
        ("evidence.rs", "is_element_factory_name", "a callee's spelling, after the `well_known` gate"),
        ("interface.rs", "try_collect_variant_bag", "the declarator's own name"),
        ("mod.rs", "skip_local_component", "the declaration's own name, for a diagnostic"),
        ("mod.rs", "scoped_key", "builds a type key from a declaration's own name"),
        ("mod.rs", "may_be_react", "a type name's spelling, Plain-only (types are never peeled)"),
        ("mod.rs", "classify_extends", "a heritage type name, Plain-only"),
        ("mod.rs", "is_constant_name", "a predicate on a declaration's own name"),
        ("uses.rs", "local", "builds `Subject::Local` from a vouched `Named`'s text or a plain binding"),
        ("uses.rs", "trusted_member", "the vouching function itself: returns a `Named` through `Bindings::named`"),
        ("wrapper.rs", "try_record_wrapper_call", "the declarator's own name"),
        ("wrapper.rs", "try_record_tagged_factory", "the declarator's own name"),
        ("wrapper.rs", "record_bound_function_result", "the declarator's own name"),
        ("wrapper.rs", "try_record_factory_call", "the declarator's own name; the callee is a `Named`"),
        ("written.rs", "find", "a written type's member name, Plain-only"),
    ];

    const MODULES: &[&str] =
        &["component.rs", "evidence.rs", "interface.rs", "mod.rs", "uses.rs", "visit.rs", "wrapper.rs", "written.rs"];

    #[test]
    fn no_extractor_function_takes_a_bare_name_a_peeled_candidate_could_pass() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/extractor");
        let names =
            ["name", "parent", "callee", "member", "alias", "target", "local", "inner", "root", "sub", "object"];
        let bare = ["&str", "String", "CompactString", "&CompactString"];
        let mut used = vec![false; ALLOWED.len()];
        let mut found = Vec::new();
        for module in MODULES {
            let source = std::fs::read_to_string(dir.join(module)).expect("an extractor source");
            let mut rest = source.as_str();
            while let Some(at) = rest.find("fn ") {
                let before = &rest[..at];
                let public = before.ends_with("pub(super) ") || before.ends_with("pub(crate) ");
                rest = &rest[at + 3..];
                let Some(open) = rest.find('(') else { break };
                let function = rest[..open].split('<').next().unwrap_or_default().trim().to_owned();
                let Some(close) = rest[open..].find(')') else { break };
                let params = &rest[open + 1..open + close];
                if !public || function.is_empty() || function.contains(char::is_whitespace) {
                    continue;
                }
                let takes_bare = params.split(',').any(|param| {
                    let Some((pname, ty)) = param.split_once(':') else { return false };
                    let ty = ty.trim().replace("'static ", "").replace("'a ", "").replace("'s ", "");
                    names.contains(&pname.trim()) && bare.contains(&ty.as_str())
                });
                if !takes_bare {
                    continue;
                }
                match ALLOWED.iter().position(|(file, allowed, _)| file == module && *allowed == function) {
                    Some(index) => used[index] = true,
                    None => found.push(format!("{module}: fn {function}({params})")),
                }
            }
        }
        assert!(found.is_empty(), "functions taking a bare name, not a `Named`:\n{}", found.join("\n"));
        let stale: Vec<_> = ALLOWED.iter().zip(&used).filter(|(_, used)| !**used).map(|(entry, _)| entry).collect();
        assert!(stale.is_empty(), "allow-list entries that match nothing: {stale:?}");
    }
}
