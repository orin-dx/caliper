//! The modules a file re-exports, without the rest of `parse_file`'s work.

use camino::Utf8Path;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::Visit;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;

use super::{MAX_SOURCE_NESTING_DEPTH, js_text, max_bracket_nesting_depth};

/// The specifiers of the modules `source` re-exports, in the order `parse_file` records them in `SourceData::exports`:
/// `export * from`, `export * as ns from`, `export { a } from` (once per name), and `export { a }` where `a` is an
/// import (`import { a } from "./a"; export { a }`).
///
/// The dependency loader follows a package's re-exports to the files that declare things and parses in full only
/// those; the files it passes through need nothing else from a parse.
pub(crate) fn re_export_specifiers(path: &Utf8Path, source: &str) -> Vec<String> {
    if max_bracket_nesting_depth(source) > MAX_SOURCE_NESTING_DEPTH {
        return Vec::new();
    }
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap_or_default();
    let options = ParseOptions { preserve_parens: false, ..ParseOptions::default() };
    let ret = Parser::new(&allocator, source, source_type).with_options(options).parse();
    let mut scan = Scan::default();
    scan.visit_program(&ret.program);
    scan.finish()
}

/// A re-export in the order its statement appears; a local one waits for the imports to be known.
enum Found {
    From(String),
    Local(String),
}

#[derive(Default)]
struct Scan {
    found: Vec<Found>,
    /// `(local name, specifier)` of each imported binding, in source order.
    imports: Vec<(String, String)>,
}

impl Scan {
    fn finish(self) -> Vec<String> {
        let Scan { found, imports } = self;
        found
            .into_iter()
            .filter_map(|item| match item {
                Found::From(specifier) => Some(specifier),
                Found::Local(local) => {
                    imports.iter().find(|(name, _)| *name == local).map(|(_, specifier)| specifier.clone())
                }
            })
            .collect()
    }
}

impl<'a> Visit<'a> for Scan {
    fn visit_import_declaration(&mut self, node: &ImportDeclaration<'a>) {
        let specifier = js_text::lossy(node.source.value);
        for spec in node.specifiers.iter().flatten() {
            let local = match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => &s.local,
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
            };
            self.imports.push((local.name.as_str().to_owned(), specifier.clone().into_owned()));
        }
    }

    fn visit_export_named_declaration(&mut self, node: &ExportNamedDeclaration<'a>) {
        for spec in &node.specifiers {
            self.found.push(Found::Local(spec.local.name().as_str().to_owned()));
        }
    }

    /// `export default Name` re-exports an import the way `export { Name as default }` does.
    fn visit_export_default_declaration(&mut self, node: &ExportDefaultDeclaration<'a>) {
        let local = match &node.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(func) => func.id.as_ref().map(|id| id.name.as_str()),
            ExportDefaultDeclarationKind::ClassDeclaration(class) => class.id.as_ref().map(|id| id.name.as_str()),
            kind => match kind.as_expression().map(super::component::peel_expr) {
                Some(Expression::Identifier(id)) => Some(id.name.as_str()),
                _ => None,
            },
        };
        if let Some(name) = local {
            self.found.push(Found::Local(name.to_owned()));
        }
    }

    fn visit_export_from_declaration(&mut self, node: &ExportFromDeclaration<'a>) {
        let specifier = js_text::lossy(node.source.value);
        for _ in &node.specifiers {
            self.found.push(Found::From(specifier.clone().into_owned()));
        }
    }

    fn visit_export_all_declaration(&mut self, node: &ExportAllDeclaration<'a>) {
        self.found.push(Found::From(js_text::lossy(node.source.value).into_owned()));
    }

    // Imports and exports are statements of a module or of an ambient module's block, never inside these.
    fn visit_function(&mut self, _: &Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}
    fn visit_class(&mut self, _: &Class<'a>) {}
    fn visit_variable_declaration(&mut self, _: &VariableDeclaration<'a>) {}
    fn visit_ts_interface_declaration(&mut self, _: &TSInterfaceDeclaration<'a>) {}
    fn visit_ts_type_alias_declaration(&mut self, _: &TSTypeAliasDeclaration<'a>) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::parse_file;
    use crate::types::LexedExport;

    /// What `parse_file` records as re-exports, in order.
    fn recorded(path: &str, source: &str) -> Vec<String> {
        parse_file(Utf8Path::new(path), source)
            .exports
            .iter()
            .filter_map(|export| match export {
                LexedExport::ReExportNamed { source_specifier, .. }
                | LexedExport::ReExportAll { source_specifier, .. }
                | LexedExport::ReExportNamespace { source_specifier, .. } => Some(source_specifier.clone()),
                LexedExport::LocalDeclaration { .. } => None,
            })
            .collect()
    }

    const SOURCES: &[(&str, &str)] = &[
        (
            "index.d.ts",
            "export * from './a';\nexport * as ns from './b';\nexport { X, Y as Z } from './c';\nexport type { T } from './d';",
        ),
        (
            "index.d.ts",
            "import { A } from './a';\nimport * as B from './b';\nimport C from './c';\nexport { A, B as Bee, C };\nexport { Local };\ndeclare const Local: number;",
        ),
        ("index.d.ts", "export {} from './empty';\nexport * from './after';"),
        ("index.d.ts", "export { A };\nimport { A } from './late';"),
        ("index.d.ts", "import { A } from './first';\nimport { A as B } from './second';\nexport { A, B };"),
        (
            "index.d.ts",
            "declare module 'x' {\n  import { P } from './p';\n  export * from './inner';\n  export { P };\n}\nexport * from './outer';",
        ),
        (
            "index.d.ts",
            "declare namespace N { export const a: number; }\nexport interface I { a: string }\nexport function f(): void;\nexport * from './g';",
        ),
        (
            "index.ts",
            "export { default } from './d';\nexport { default as Other } from './e';\nexport const x = () => 1;",
        ),
        ("index.d.ts", "import Tooltip from './tooltip';\nexport default Tooltip;"),
        ("index.d.ts", "import Tooltip from './tooltip';\nexport default (Tooltip as any);\nexport * from './z';"),
        ("index.d.ts", "import { A } from './a';\nexport default function A() {}"),
        ("index.d.ts", "export * from './\\ud800';"),
        ("index.d.ts", "this is not typescript {{{ export * from './recovered'"),
        ("index.d.ts", ""),
    ];

    #[test]
    fn it_finds_the_re_exports_parse_file_records_in_the_same_order() {
        for (path, source) in SOURCES {
            assert_eq!(re_export_specifiers(Utf8Path::new(path), source), recorded(path, source), "{source}");
        }
    }

    #[test]
    fn a_file_nested_past_the_limit_has_none() {
        let source = format!("export * from './a';\ntype T = {};", "(".repeat(MAX_SOURCE_NESTING_DEPTH + 1));
        assert_eq!(re_export_specifiers(Utf8Path::new("index.d.ts"), &source), recorded("index.d.ts", &source));
        assert!(re_export_specifiers(Utf8Path::new("index.d.ts"), &source).is_empty());
    }
}
