//! What oxc reads from the source for the checker's answer (ADR 0026, S7a2): the defaults a component's own code gives
//! its props, and the order a prop's declaration writes its union in. The checker has neither (it keeps no parameter
//! initializer, and it sorts a union's members), and the structural resolver that supplied both goes in S7e.
//!
//! The files are the ones the answer names: the component's file, for its defaults and for the props a type literal
//! declares, and each union prop's `parent` file. Each is read and parsed once per merge.

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use super::SemanticProp;
use crate::extractor::written::{WrittenMember, WrittenUnions, written_unions};
use crate::types::collected::ComponentExport;
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
    /// Each component the extractor finds in the file: its name, its exports and its code defaults.
    components: Vec<(String, Vec<ComponentExport>, FxHashMap<String, DefaultValue>)>,
    unions: WrittenUnions,
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
        for (entry, props) in components {
            wanted.entry(entry.file_path.clone()).or_default().defaults = true;
            for prop in props.iter().filter(|prop| matches!(prop.prop_type, PropType::Union(_))) {
                let file = match &prop.parent {
                    Some(parent) => Utf8PathBuf::from(&parent.file_name),
                    None => entry.file_path.clone(),
                };
                wanted.entry(file).or_default().unions = true;
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
        let file = self.files.get(&entry.file_path)?;
        let mut found = file
            .components
            .iter()
            .filter(|(name, exports, _)| *name == entry.display_name && *exports == entry.exports)
            .map(|(_, _, defaults)| defaults);
        let first = found.next()?;
        found.all(|other| other == first).then_some(first)
    }

    /// The union `prop`'s declaration writes: in its `parent` interface, or for a prop with no parent, in a type literal
    /// of `entry`'s file. `None` when the file writes none for it, or two different ones.
    pub(super) fn written(&self, entry: &ComponentEntry, prop: &SemanticProp) -> Option<&[WrittenMember]> {
        match &prop.parent {
            Some(parent) => {
                self.files.get(Utf8Path::new(&parent.file_name))?.unions.find(Some(&parent.name), &prop.name)
            }
            None => self.files.get(&entry.file_path)?.unions.find(None, &prop.name),
        }
    }
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
            .map(|mapping| {
                let defaults = mapping
                    .param_defaults
                    .into_iter()
                    .map(|(name, raw)| (name, DefaultValue { value: raw.value, computed: raw.computed }))
                    .collect();
                (mapping.component_name, mapping.exports, defaults)
            })
            .collect();
    }
    if wanted.unions {
        file.unions = written_unions(path, &source);
    }
    Ok(file)
}
