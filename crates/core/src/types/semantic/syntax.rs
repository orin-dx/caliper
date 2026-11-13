//! What oxc reads from the source for the checker's answer (ADR 0026, S7a2): the defaults a component's own code gives
//! its props, and the order a prop's declaration writes its union in. The checker has neither (it keeps no parameter
//! initializer, and it sorts a union's members), and the structural resolver that supplied both goes in S7e.
//!
//! The files are the ones the answer names: the component's file, for its defaults and for the props a type literal
//! declares, and each union prop's `parent` file. Each is read and parsed once per merge.

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use super::SemanticProp;
use crate::extractor::written::{MAX_WRITTEN_MEMBERS, Owner, WrittenMember, WrittenUnions, written_unions};
use crate::types::collected::{ComponentExport, TypeName};
use crate::types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
use crate::types::output::{ComponentEntry, DefaultValue, PropType};

/// What a merge reads from one file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Wanted {
    /// The defaults of the components it declares.
    defaults: bool,
    /// The unions its property signatures write.
    unions: bool,
}

#[derive(Debug, Default)]
struct FileSyntax {
    components: Vec<ComponentSyntax>,
    unions: WrittenUnions,
}

/// A component the extractor finds in a file.
#[derive(Debug)]
struct ComponentSyntax {
    name: String,
    exports: Vec<ComponentExport>,
    defaults: FxHashMap<String, DefaultValue>,
    /// The name of the type its props are, as the declaration writes it.
    props_type: TypeName,
    /// The bytes of its declaration: a type literal written in an annotation there is (part of) its props type.
    span: std::ops::Range<u32>,
}

impl ComponentSyntax {
    /// Whether a type literal owned by `owner` is (part of) this component's props type: the body of the module-level
    /// alias its declaration names, or written in its declaration.
    fn holds(&self, owner: &Owner) -> bool {
        match owner {
            Owner::Alias(alias) => *alias == self.props_type,
            Owner::Annotation(start) => self.span.contains(start),
            Owner::Interface(_) | Owner::Elsewhere => false,
        }
    }
}

/// The syntax of the files one merge needs, read up front.
#[derive(Debug, Default)]
pub(super) struct SourceSyntax {
    files: FxHashMap<Utf8PathBuf, FileSyntax>,
    /// A file it could not read, other than one that does not exist on disk (a checker's bundled lib file).
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl SourceSyntax {
    /// Reads what the checker's `props` of each entry need: the entry's file, and the `parent` file of each prop whose
    /// type is a union. Files are read in parallel.
    pub(super) fn read<'a>(components: impl Iterator<Item = (&'a ComponentEntry, &'a [Arc<SemanticProp>])>) -> Self {
        let mut wanted: FxHashMap<Utf8PathBuf, Wanted> = FxHashMap::default();
        // The checker's answer holds the same few thousand props under every component that inherits them, so a prop
        // already seen, and a file already named, adds nothing: each is skipped without building a path. A file is
        // still first inserted at its first sighting, so `wanted` iterates as it did when every prop inserted.
        let mut seen_props: FxHashSet<usize> = FxHashSet::default();
        let mut seen_files: FxHashSet<&str> = FxHashSet::default();
        for (entry, props) in components {
            wanted.entry(entry.file_path.clone()).or_default().defaults = true;
            let mut own_file = false;
            for prop in props.iter().filter(|prop| matches!(prop.prop_type, PropType::Union(_))) {
                match &prop.parent {
                    Some(parent) => {
                        if seen_props.insert(Arc::as_ptr(prop) as usize) && seen_files.insert(parent.file_name.as_str())
                        {
                            wanted.entry(Utf8PathBuf::from(&parent.file_name)).or_default().unions = true;
                        }
                    }
                    None if !own_file => {
                        own_file = true;
                        wanted.entry(entry.file_path.clone()).or_default().unions = true;
                    }
                    None => {}
                }
            }
        }
        let read: Vec<(Utf8PathBuf, Result<FileSyntax, Option<Diagnostic>>)> = wanted
            .into_par_iter()
            .map(|(path, wanted)| {
                let file = read_file(&path, wanted);
                (path, file)
            })
            .collect();
        let mut syntax = SourceSyntax::default();
        for (path, file) in read {
            match file {
                Ok(file) => {
                    if file.unions.declined > 0 {
                        syntax.diagnostics.push(too_many_members(&path, file.unions.declined));
                    }
                    syntax.files.insert(path, file);
                }
                Err(diagnostic) => syntax.diagnostics.extend(diagnostic),
            }
        }
        syntax
    }

    /// The defaults `entry`'s own code gives its props: those of the component the extractor finds in its file under
    /// its name and exports. `None` when the file has no such component, or several with different defaults.
    pub(super) fn code_defaults(&self, entry: &ComponentEntry) -> Option<&FxHashMap<String, DefaultValue>> {
        let mut found = self.components(entry)?.map(|component| &component.defaults);
        let first = found.next()?;
        found.all(|other| other == first).then_some(first)
    }

    /// The components of `entry`'s file the extractor finds under its name and exports.
    fn components<'s, 'e>(
        &'s self,
        entry: &'e ComponentEntry,
    ) -> Option<impl Iterator<Item = &'s ComponentSyntax> + use<'s, 'e>> {
        let file = self.files.get(&entry.file_path)?;
        Some(
            file.components
                .iter()
                .filter(|component| component.name == entry.display_name && component.exports == entry.exports),
        )
    }

    /// The union `prop`'s declaration writes: in its `parent` interface, or for a prop with no parent, in a type literal
    /// of `entry`'s file that is (part of) its component's props type. `None` when the file writes none for it, or two
    /// different ones: a type literal the props type cannot be shown to be (an imported props type) says nothing.
    pub(super) fn written(&self, entry: &ComponentEntry, prop: &SemanticProp) -> Option<&[WrittenMember]> {
        match &prop.parent {
            Some(parent) => self
                .files
                .get(Utf8Path::new(&parent.file_name))?
                .unions
                .find(&prop.name, |owner| matches!(owner, Owner::Interface(name) if *name == parent.name)),
            None => {
                let components: Vec<&ComponentSyntax> = self.components(entry)?.collect();
                self.files
                    .get(&entry.file_path)?
                    .unions
                    .find(&prop.name, |owner| components.iter().any(|component| component.holds(owner)))
            }
        }
    }
}

/// The diagnostic for the `declined` unions of `path` that state more members than are read.
fn too_many_members(path: &Utf8Path, declined: usize) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        DiagnosticSeverity::Info,
        DiagnosticCode::MaxDepthExceeded,
        format!(
            "{declined} union(s) in '{path}' state more than {MAX_WRITTEN_MEMBERS} members through the aliases they \
             name: their written order is not read, and their members keep the order the merge had otherwise"
        ),
    );
    diagnostic.file = Some(path.to_string());
    diagnostic
}

fn read_file(path: &Utf8Path, wanted: Wanted) -> Result<FileSyntax, Option<Diagnostic>> {
    let source = std::fs::read_to_string(path).map_err(|error| {
        (error.kind() != std::io::ErrorKind::NotFound).then(|| {
            let mut diagnostic = Diagnostic::new(
                DiagnosticSeverity::Warning,
                DiagnosticCode::SemanticBackend,
                format!(
                    "failed to read '{path}' for the defaults and union member order of the checker's answer: {error}"
                ),
            );
            diagnostic.file = Some(path.to_string());
            diagnostic
        })
    })?;
    let mut file = FileSyntax::default();
    if wanted.defaults {
        file.components = crate::extractor::parse_file(path, &source)
            .component_mappings
            .into_iter()
            .map(|mapping| ComponentSyntax {
                defaults: mapping
                    .param_defaults
                    .into_iter()
                    .map(|(name, raw)| (name, DefaultValue { value: raw.value, computed: raw.computed }))
                    .collect(),
                name: mapping.component_name,
                exports: mapping.exports,
                props_type: mapping.props_type_name,
                span: mapping.span_start..mapping.span_end,
            })
            .collect();
    }
    if wanted.unions {
        file.unions = written_unions(path, &source);
    }
    Ok(file)
}
