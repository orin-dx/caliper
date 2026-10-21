//! OXC AST extractor — Phase 2a.
//!
//! Walks the OXC AST for a single file and collects everything into a [`SourceData`].
//!
//! **The Absolute Rule:** No AST references escape `parse_file`.
//! The `Allocator` is created locally, and all arena memory is freed when `parse_file` returns.
//! [`SourceData`] contains only owned data (String, Vec, etc.).

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use oxc_allocator::{Allocator, Box as OxcBox};
use oxc_ast::ast::*;
use oxc_ast_visit::Visit;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{SourceType, Span};
use rustc_hash::FxHashSet;

#[cfg(test)]
use crate::types::LexedExport;
use crate::types::{
    CollectedObjectField, CollectedType, CollectedTypeAlias, Diagnostic, DiagnosticCode, DiagnosticSeverity,
    ExtendsRef, RawProp, SourceData,
};

mod alias;
mod component;
mod defaults;
mod evidence;
mod interface;
mod jsdoc;
mod visit;
mod wrapper;

// ─── Entry Point ─────────────────────────────────────────────────────────────

/// Maximum bracket-nesting depth a source file may contain before we refuse to parse it.
///
/// `oxc_parser`'s recursive-descent grammar has no depth guard for nested parenthesized
/// types, object type literals, or conditional types — a file with ~6,000+ levels of
/// nesting deterministically stack-overflows the parser itself (confirmed via a
/// standalone parser-only harness). 2000 leaves a wide safety margin.
const MAX_SOURCE_NESTING_DEPTH: usize = 2000;

/// Maximum AST recursion depth for `ts_type_to_collected` and its mutually
/// recursive siblings. `max_bracket_nesting_depth` bounds raw-text bracket
/// depth as a cheap pre-parse proxy for parser stack safety, but chained
/// conditional types (`A extends B ? C extends D ? ... : ... : ...`) add one
/// AST level per `? :` with no brackets at all — the proxy metric undercounts
/// exactly this shape. This counter guards the extractor's own recursion the
/// same way the resolver's `depth: u8` / `MAX_DEPTH` already guards
/// `resolve_collected_type` (see `resolver/mod.rs`), just at a higher ceiling
/// since this walk is a single in-process AST-to-struct conversion, not
/// cross-file resolution. `MAX_SOURCE_NESTING_DEPTH` (bracket-based) and this
/// constant (AST-based) are independent knobs and do not need to match.
const MAX_TYPE_COLLECT_DEPTH: u8 = 200;

/// Cheap linear scan bounding the maximum bracket-nesting depth of `source`.
///
/// Only tracks a running max, not full balance — sufficient to bound recursion depth
/// before handing the source to the real parser. Comments and string/template
/// literals are skipped entirely rather than scanned for brackets: real .d.ts
/// files (TypeScript's own `lib.dom.d.ts` included) ship prose JSDoc with
/// unmatched brackets — e.g. MDN-scraped artifacts like `MISSING: RFC(5646,
/// '...')].` — that would otherwise drive `depth` negative and, once negative
/// enough, wrap on the next legitimate bracket. Doesn't track `${...}`
/// interpolation inside template literals as real nesting; under-counting a
/// rare template-literal type's internal depth is an acceptable trade-off for
/// a crash-prevention heuristic, never causing a false rejection.
fn max_bracket_nesting_depth(source: &str) -> usize {
    let bytes = source.as_bytes();
    let mut depth: usize = 0;
    let mut max_depth: usize = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            quote @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += if bytes[i] == b'\\' && i + 1 < bytes.len() { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
            }
            b'(' | b'{' | b'[' => {
                depth += 1;
                max_depth = max_depth.max(depth);
                i += 1;
            }
            b')' | b'}' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    max_depth
}

/// Parse a single file and collect all extractable data.
///
/// Completely pure — no I/O, no side effects, no cross-file dependencies.
/// Safe to call in parallel from rayon workers.
///
/// Never panics on malformed input: excessive nesting and TypeScript syntax errors
/// are reported via `SourceData::diagnostics` rather than failing silently or
/// letting the parser overrun the stack.
pub fn parse_file(path: &Utf8Path, source: &str) -> SourceData {
    let observed_depth = max_bracket_nesting_depth(source);
    if observed_depth > MAX_SOURCE_NESTING_DEPTH {
        let mut data = SourceData::default();
        data.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "File exceeds maximum type nesting depth ({observed_depth} > {MAX_SOURCE_NESTING_DEPTH}), skipped to avoid parser stack overflow"
            ),
            file: Some(path.to_string()),
            line: None,
            column: None,
            help: Some("Reduce nested parenthesized/object/conditional types in this file.".into()),
            code: DiagnosticCode::ExcessiveNesting,
            affects: Vec::new(),
        });
        return data;
    }

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap_or_default();
    // Parentheses carry no meaning here, and a `ParenthesizedExpression` node would hide the wrapper call beneath it.
    let options = ParseOptions { preserve_parens: false, ..ParseOptions::default() };
    let ret = Parser::new(&allocator, source, source_type).with_options(options).parse();

    // .d.ts declaration files use the same React.FC / ForwardRefExoticComponent patterns as .tsx
    let is_tsx = source_type.is_jsx() || source_type.is_typescript_definition();
    let mut collector = SourceDataCollector::new(path, source, is_tsx);

    // oxc_parser recovers from errors: `ret.program` is usable even with diagnostics, so surface them rather than
    // treat the source as clean.
    for err in &ret.diagnostics {
        collector.data.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            file: Some(path.to_string()),
            line: None,
            column: None,
            help: None,
            code: DiagnosticCode::ParseError,
            affects: Vec::new(),
        });
    }

    // Pass comments by cloning them into owned strings before the allocator drops.
    // The comments Vec lives in the arena; we extract them here.
    let comments: Vec<OwnedComment> = ret
        .program
        .comments
        .iter()
        .map(|c| OwnedComment { span_start: c.span.start, span_end: c.span.end, is_block: c.is_block() })
        .collect();
    collector.comments = comments;
    collector.visit_program(&ret.program);
    collector.finish()
    // allocator drops here — ALL arena memory freed
}

// ─── Owned Comment (so nothing escapes the allocator) ────────────────────────

#[derive(Debug, Clone)]
pub(super) struct OwnedComment {
    pub(super) span_start: u32,
    pub(super) span_end: u32,
    pub(super) is_block: bool,
}

// ─── Collector ───────────────────────────────────────────────────────────────

pub(super) struct SourceDataCollector<'src> {
    pub(super) file_path: Utf8PathBuf,
    pub(super) source: &'src str,
    pub(super) is_tsx: bool,
    pub(super) data: SourceData,
    /// All comments in the file (owned — not tied to allocator lifetime).
    pub(super) comments: Vec<OwnedComment>,
    /// Names that came from imports — used to classify ExtendsRef.
    pub(super) imported_names: FxHashSet<CompactString>,
    /// Tracks which JSDoc comment spans have already been consumed (by span_end).
    /// Prevents the same comment from leaking to both a component description and its first prop.
    pub(super) consumed_jsdoc: FxHashSet<u32>,
    /// Enclosing `namespace X { ... }` names, outermost first. Type references to a
    /// namespace member are always fully qualified (`X.Y`, per `ts_type_name_str`'s
    /// `TSTypeName::QualifiedName` handling) — so storage keys must match, or a same-file
    /// reference to a namespace member can never resolve.
    pub(super) namespace_stack: Vec<CompactString>,
    /// `X.displayName = "..."` renames recorded during the main traversal but
    /// not yet applied — see `try_scan_display_name`. Applying these
    /// immediately would change `component_name` out from under any later
    /// static-property scan (`try_scan_default_props`, another displayName
    /// assignment) still looking the mapping up by its original identifier;
    /// deferring to `finish()` means every other scan always sees the
    /// original identifier for the whole file, regardless of source order.
    pub(super) pending_display_name_renames: Vec<(std::string::String, std::string::String)>,
    /// Identifiers that were the *source* of a `try_rename_identifier_wrapped_component`
    /// alias — e.g. `InternalButton` in `const Button = InternalButton;`. Once
    /// aliased, the original is implementation detail, not a second public
    /// component; filtered out of the final `component_mappings` in `finish()`.
    pub(super) aliased_away: FxHashSet<CompactString>,
    /// Wrapper-call candidates, promoted to mappings in `finish()` once the whole file has been seen.
    pub(super) pending_wrappers: Vec<wrapper::PendingWrapper>,
    /// `export { local as exported }` without a source: the index into `data.exports` and the `local` name, so
    /// `finish()` can recognize a re-exported import once every import has been seen.
    pub(super) pending_local_exports: Vec<(usize, String)>,
    /// Why the current declarator's detector chain declined a candidate, used as the skip diagnostic's message.
    pub(super) decline_reason: Option<String>,
}

impl<'src> SourceDataCollector<'src> {
    fn new(path: &Utf8Path, source: &'src str, is_tsx: bool) -> Self {
        Self {
            file_path: path.to_owned(),
            source,
            is_tsx,
            data: SourceData::default(),
            comments: Vec::new(),
            imported_names: FxHashSet::default(),
            consumed_jsdoc: FxHashSet::default(),
            namespace_stack: Vec::new(),
            pending_display_name_renames: Vec::new(),
            aliased_away: FxHashSet::default(),
            pending_wrappers: Vec::new(),
            pending_local_exports: Vec::new(),
            decline_reason: None,
        }
    }

    /// Record that a recognized-but-malformed AST shape was skipped — distinct
    /// from "wrong shape, not a candidate at all" (which stays silent). Used by
    /// `classify_type_alias`'s Omit/Pick/Partial/Required/Readonly arms and the
    /// component-detector chains in `visit.rs` when a shape matches a known
    /// pattern but is missing/malformed pieces the pattern requires.
    pub(super) fn record_skip(&mut self, code: DiagnosticCode, message: impl Into<String>, span: Span) {
        let (line, column) = line_and_column(self.source, span.start);
        self.data.diagnostics.push(
            Diagnostic::new(DiagnosticSeverity::Info, code, message)
                .with_file(self.file_path.to_string())
                .with_position(line, column),
        );
    }

    pub(super) fn scoped_key(&self, name: &str) -> String {
        if self.namespace_stack.is_empty() {
            format!("{}:{}", self.file_path, name)
        } else {
            format!("{}:{}.{}", self.file_path, self.namespace_stack.join("."), name)
        }
    }

    fn finish(mut self) -> SourceData {
        self.promote_wrapper_calls();
        self.promote_reexported_imports();
        // Filtered by original identifier first, before any displayName rename
        // below can change what identifier a mapping currently answers to.
        if !self.aliased_away.is_empty() {
            // An exported base is public under its own name too (`export function Card`, `export const MemoCard =
            // memo(Card)`); only an implementation-only base is hidden behind its alias.
            let exported: rustc_hash::FxHashSet<&str> = self
                .data
                .exports
                .iter()
                .filter_map(|e| match e {
                    crate::types::LexedExport::LocalDeclaration { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            let hidden: FxHashSet<CompactString> =
                self.aliased_away.iter().filter(|n| !exported.contains(n.as_str())).cloned().collect();
            self.data.component_mappings.retain(|m| !hidden.contains(m.component_name.as_str()));
        }
        for (obj_name, display_name) in self.pending_display_name_renames.drain(..) {
            if let Some(mapping) = self.data.component_mappings.iter_mut().find(|m| m.component_name == obj_name) {
                mapping.component_name = display_name;
            }
        }
        self.data
    }

    /// `import { A } from "./a"; export { A }` re-exports `A` exactly like `export { A } from "./a"`; barrels use both
    /// spellings, and only the second names a source file to follow.
    fn promote_reexported_imports(&mut self) {
        for (index, local) in std::mem::take(&mut self.pending_local_exports) {
            let Some(binding) = self.data.imports.iter().find(|imp| imp.local_name.as_str() == local) else { continue };
            if binding.exported_name.as_str() == "*" {
                continue;
            }
            let crate::types::LexedExport::LocalDeclaration { name, is_type_only } = &self.data.exports[index] else {
                continue;
            };
            self.data.exports[index] = crate::types::LexedExport::ReExportNamed {
                local_name: name.clone(),
                source_name: binding.exported_name.to_string(),
                source_specifier: binding.specifier.clone(),
                is_type_only: *is_type_only || binding.is_type_only,
            };
        }
    }

    // ─── Import source specifier lookup ──────────────────────────────────────

    pub(super) fn find_import_specifier(&self, local_name: &str) -> Option<String> {
        self.data.imports.iter().find(|imp| imp.local_name.as_str() == local_name).map(|imp| imp.specifier.clone())
    }

    // ─── ExtendsRef classification ────────────────────────────────────────────

    pub(super) fn classify_extends(&self, name: &str, type_args: Vec<CollectedType>) -> ExtendsRef {
        // Strip "React." prefix for lookup in builtin tables
        let lookup_name = name.strip_prefix("React.").unwrap_or(name);

        // SVGAttributes/SVGProps/HTMLProps carry no element in their own name —
        // unlike html_element_for's other entries (ButtonHTMLAttributes, etc.),
        // where the element is baked into the interface name itself. Real call
        // sites always supply a concrete DOM element as the type argument
        // (`SVGAttributes<SVGSVGElement>`, `HTMLProps<HTMLDivElement>`) — derive
        // the element from that instead of falling through to html_element_for's
        // static result (None for SVGAttributes/SVGProps, a generic "div"
        // fallback for HTMLProps) whenever it's one this crate recognizes.
        if matches!(lookup_name, "SVGAttributes" | "SVGProps" | "HTMLProps") {
            if let Some(element) = type_args.first().and_then(crate::react_types::html_element_from_collected) {
                return ExtendsRef::Builtin { name: name.into(), element: Some(element.to_owned()), type_args };
            }
        }

        if let Some(element) = crate::react_types::html_element_for(lookup_name) {
            return ExtendsRef::Builtin { name: name.into(), element: Some(element.to_owned()), type_args };
        }
        if crate::react_types::is_react_builtin(lookup_name, &rustc_hash::FxHashSet::default()) {
            return ExtendsRef::Builtin { name: name.into(), element: None, type_args };
        }
        // Check if imported
        if self.imported_names.contains(name) {
            return ExtendsRef::Imported {
                local_name: name.into(),
                type_args,
                source_specifier: self.find_import_specifier(name),
            };
        }
        ExtendsRef::SameFile { name: name.into(), type_args }
    }

    // ─── TSTypeName → String ──────────────────────────────────────────────────

    pub(super) fn ts_type_name_str<'a>(&self, name: &TSTypeName<'a>) -> String {
        match name {
            TSTypeName::IdentifierReference(id) => id.name.as_str().to_owned(),
            TSTypeName::QualifiedName(q) => {
                format!("{}.{}", self.ts_type_name_str(&q.left), q.right.name.as_str())
            }
            TSTypeName::ThisExpression(_) => "this".to_owned(),
        }
    }

    // ─── TSTypeParameterInstantiation → Vec<CollectedType> ───────────────────

    pub(super) fn extract_type_args<'a>(
        &mut self,
        type_params: &Option<OxcBox<'a, TSTypeParameterInstantiation<'a>>>,
    ) -> Vec<CollectedType> {
        match type_params {
            Some(tp) => tp.params.iter().map(|p| self.ts_type_to_collected_at_depth(p, 0)).collect(),
            None => vec![],
        }
    }

    // ─── TSType → CollectedType ───────────────────────────────────────────────

    pub(super) fn ts_type_to_collected<'a>(&mut self, ty: &TSType<'a>) -> CollectedType {
        self.ts_type_to_collected_at_depth(ty, 0)
    }

    fn ts_type_to_collected_at_depth<'a>(&mut self, ty: &TSType<'a>, depth: u8) -> CollectedType {
        if depth > MAX_TYPE_COLLECT_DEPTH {
            use oxc_span::GetSpan;
            let span = ty.span();
            self.data.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "Type nesting exceeds maximum extractor recursion depth ({depth} > {MAX_TYPE_COLLECT_DEPTH})"
                ),
                file: Some(self.file_path.to_string()),
                line: None,
                column: None,
                help: Some("This may indicate a deeply chained conditional or mapped type.".into()),
                code: DiagnosticCode::MaxDepthExceeded,
                affects: Vec::new(),
            });
            let raw = self.source[span.start as usize..span.end as usize].to_owned();
            return CollectedType::Raw(raw);
        }
        match ty {
            TSType::TSStringKeyword(_) => CollectedType::String,
            TSType::TSNumberKeyword(_) => CollectedType::Number,
            TSType::TSBooleanKeyword(_) => CollectedType::Boolean,
            TSType::TSNullKeyword(_) => CollectedType::Null,
            TSType::TSUndefinedKeyword(_) => CollectedType::Undefined,
            TSType::TSAnyKeyword(_) => CollectedType::Any,
            TSType::TSNeverKeyword(_) => CollectedType::Never,
            TSType::TSUnknownKeyword(_) => CollectedType::Unknown,
            TSType::TSVoidKeyword(_) => CollectedType::Void,
            TSType::TSBigIntKeyword(_) => CollectedType::BigInt,
            TSType::TSSymbolKeyword(_) => CollectedType::Symbol,
            TSType::TSObjectKeyword(_) => CollectedType::Named { name: "object".into(), args: vec![] },

            TSType::TSLiteralType(lit) => match &lit.literal {
                TSLiteral::StringLiteral(s) => CollectedType::StringLiteral(s.value.as_str().into()),
                TSLiteral::NumericLiteral(n) => CollectedType::NumberLiteral(n.value),
                TSLiteral::BooleanLiteral(b) => CollectedType::BoolLiteral(b.value),
                TSLiteral::UnaryExpression(u) => {
                    // Handle negative numbers: -1
                    let raw = self.source[u.span.start as usize..u.span.end as usize].to_owned();
                    CollectedType::Raw(raw)
                }
                _ => CollectedType::Raw(self.source[lit.span.start as usize..lit.span.end as usize].to_owned()),
            },

            TSType::TSTypeReference(tr) => {
                let name: CompactString = self.ts_type_name_str(&tr.type_name).into();
                let args = tr
                    .type_arguments
                    .as_ref()
                    .map(|ta| ta.params.iter().map(|p| self.ts_type_to_collected_at_depth(p, depth + 1)).collect())
                    .unwrap_or_default();
                CollectedType::Named { name, args }
            }

            TSType::TSTypeQuery(q) => {
                let name = self.ts_type_query_name(q);
                CollectedType::TypeOf(name.into())
            }

            TSType::TSUnionType(u) => {
                let members: Vec<CollectedType> =
                    u.types.iter().map(|t| self.ts_type_to_collected_at_depth(t, depth + 1)).collect();
                CollectedType::Union(members)
            }

            TSType::TSIntersectionType(i) => {
                let members: Vec<CollectedType> =
                    i.types.iter().map(|t| self.ts_type_to_collected_at_depth(t, depth + 1)).collect();
                CollectedType::Intersection(members)
            }

            TSType::TSArrayType(a) => {
                CollectedType::Array(Box::new(self.ts_type_to_collected_at_depth(&a.element_type, depth + 1)))
            }

            TSType::TSTupleType(t) => {
                let members: Vec<CollectedType> =
                    t.element_types.iter().map(|el| self.ts_tuple_element_to_collected(el, depth + 1)).collect();
                CollectedType::Tuple(members)
            }

            TSType::TSTypeLiteral(lit) => {
                let fields: Vec<CollectedObjectField> = lit
                    .members
                    .iter()
                    .filter_map(|member| self.ts_signature_to_object_field(member, depth + 1))
                    .collect();
                CollectedType::Object(fields)
            }

            TSType::TSFunctionType(f) => {
                // In OXC 0.135, FormalParameter has type_annotation as a separate field
                let params: Vec<CollectedType> = f
                    .params
                    .items
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|ta| self.ts_type_to_collected_at_depth(&ta.type_annotation, depth + 1))
                            .unwrap_or(CollectedType::Any)
                    })
                    .collect();
                let param_names: Vec<Option<CompactString>> =
                    f.params.items.iter().map(|p| binding_pattern_name(&p.pattern)).collect();
                // return_type on TSFunctionType is Box<TSTypeAnnotation> (not Option)
                let return_type = self.ts_type_to_collected_at_depth(&f.return_type.type_annotation, depth + 1);
                CollectedType::Function { params, param_names, return_type: Box::new(return_type) }
            }

            TSType::TSIndexedAccessType(ia) => CollectedType::IndexedAccess {
                obj: Box::new(self.ts_type_to_collected_at_depth(&ia.object_type, depth + 1)),
                key: Box::new(self.ts_type_to_collected_at_depth(&ia.index_type, depth + 1)),
            },

            TSType::TSTemplateLiteralType(tl) => {
                let mut parts: Vec<CollectedType> = Vec::new();
                for (i, quasi) in tl.quasis.iter().enumerate() {
                    let s = quasi.value.raw.as_str();
                    if !s.is_empty() {
                        parts.push(CollectedType::StringLiteral(s.into()));
                    }
                    if let Some(ty) = tl.types.get(i) {
                        parts.push(self.ts_type_to_collected_at_depth(ty, depth + 1));
                    }
                }
                CollectedType::TemplateLiteral(parts)
            }

            TSType::TSConditionalType(c) => CollectedType::Conditional {
                check: Box::new(self.ts_type_to_collected_at_depth(&c.check_type, depth + 1)),
                extends_type: Box::new(self.ts_type_to_collected_at_depth(&c.extends_type, depth + 1)),
                true_type: Box::new(self.ts_type_to_collected_at_depth(&c.true_type, depth + 1)),
                false_type: Box::new(self.ts_type_to_collected_at_depth(&c.false_type, depth + 1)),
            },

            TSType::TSMappedType(m) => {
                // In OXC 0.135, TSMappedType has `constraint: TSType` directly (not via
                // type_parameter) and `type_annotation: Option<TSType>` (not Box<TSTypeAnnotation>)
                let key_type = self.ts_type_to_collected_at_depth(&m.constraint, depth + 1);
                let value_type = m
                    .type_annotation
                    .as_ref()
                    .map(|ta| self.ts_type_to_collected_at_depth(ta, depth + 1))
                    .unwrap_or(CollectedType::Unknown);
                CollectedType::Mapped { key_type: Box::new(key_type), value_type: Box::new(value_type) }
            }

            TSType::TSParenthesizedType(p) => {
                // Unwrap parentheses — (Type) → Type
                self.ts_type_to_collected_at_depth(&p.type_annotation, depth + 1)
            }

            // TSTypeOperatorType covers keyof, unique, readonly. `keyof` is kept
            // structured (its operand may itself need substitution or resolution —
            // see `CollectedType::KeyOf`). `readonly`/`unique` are peeled
            // transparently to their operand — docgen doesn't track mutability or
            // symbol uniqueness, so there's nothing to preserve — rather than
            // capturing the whole modified type as raw source text, which used to
            // make `readonly string[]` (real @types/react's ButtonHTMLAttributes
            // `defaultValue`/`value` shape) degrade to Opaque even though the
            // element type is fully knowable.
            TSType::TSTypeOperatorType(op) => match op.operator {
                TSTypeOperatorOperator::Keyof => {
                    CollectedType::KeyOf(Box::new(self.ts_type_to_collected_at_depth(&op.type_annotation, depth + 1)))
                }
                TSTypeOperatorOperator::Unique | TSTypeOperatorOperator::Readonly => {
                    self.ts_type_to_collected_at_depth(&op.type_annotation, depth + 1)
                }
            },

            TSType::TSInferType(i) => {
                let raw = self.source[i.span.start as usize..i.span.end as usize].to_owned();
                CollectedType::Raw(raw)
            }

            // Anything else: capture raw source text as fallback
            _ => {
                use oxc_span::GetSpan;
                let span = ty.span();
                let raw = self.source[span.start as usize..span.end as usize].to_owned();
                CollectedType::Raw(raw)
            }
        }
    }

    /// Convert a `TSTupleElement` (which is a superset of `TSType`) to a `CollectedType`.
    ///
    /// TSTupleElement inherits all TSType variants and adds TSOptionalType and TSRestType.
    pub(super) fn ts_tuple_element_to_collected<'a>(&mut self, el: &TSTupleElement<'a>, depth: u8) -> CollectedType {
        match el {
            TSTupleElement::TSOptionalType(o) => {
                // T? in tuple → Union([T, Undefined])
                let inner = self.ts_type_to_collected_at_depth(&o.type_annotation, depth + 1);
                CollectedType::Union(vec![inner, CollectedType::Undefined])
            }
            TSTupleElement::TSRestType(r) => {
                // ...T[] in tuple → Array(T)
                CollectedType::Array(Box::new(self.ts_type_to_collected_at_depth(&r.type_annotation, depth + 1)))
            }
            // All TSType variants are inherited — we can safely transmute via span+raw fallback
            // but the cleanest approach is to match the known shared variants.
            // Since TSTupleElement inherits TSType, we cast via the span trick:
            other => {
                use oxc_span::GetSpan;
                let span = other.span();
                let raw = self.source[span.start as usize..span.end as usize].to_owned();
                // Try to produce a real type from source-text parsing would require re-parsing,
                // so instead we check common patterns via span-based raw parsing and use
                // the raw fallback for anything we can't directly handle.
                // For the common case of plain TSType variants (not TSOptionalType/TSRestType),
                // we need to re-interpret the element as a TSType. We can't do this safely
                // at runtime without unsafe, but all such variants produce valid CollectedType
                // via raw text. Most real tuple usage is simple types.
                CollectedType::Raw(raw)
            }
        }
    }

    pub(super) fn ts_type_query_name<'a>(&self, q: &TSTypeQuery<'a>) -> String {
        // TSTypeQueryExprName can be Identifier or a qualified name
        // Capture the source text of the expression name
        use oxc_span::GetSpan;
        let span = q.expr_name.span();
        self.source[span.start as usize..span.end as usize].to_owned()
    }

    pub(super) fn ts_signature_to_object_field<'a>(
        &mut self,
        member: &TSSignature<'a>,
        depth: u8,
    ) -> Option<CollectedObjectField> {
        match member {
            TSSignature::TSPropertySignature(sig) => {
                let name = sig.key.static_name()?.to_string();
                let collected_type = sig
                    .type_annotation
                    .as_ref()
                    .map(|ta| self.ts_type_to_collected_at_depth(&ta.type_annotation, depth + 1))
                    .unwrap_or(CollectedType::Any);
                let description = self.find_jsdoc(sig.span.start);
                Some(CollectedObjectField { name, collected_type, required: !sig.optional, description })
            }
            TSSignature::TSMethodSignature(sig) => {
                let name = sig.key.static_name()?.to_string();
                let params: Vec<CollectedType> = sig
                    .params
                    .items
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|ta| self.ts_type_to_collected_at_depth(&ta.type_annotation, depth + 1))
                            .unwrap_or(CollectedType::Any)
                    })
                    .collect();
                let param_names: Vec<Option<CompactString>> =
                    sig.params.items.iter().map(|p| binding_pattern_name(&p.pattern)).collect();
                let return_type = sig
                    .return_type
                    .as_ref()
                    .map(|rt| self.ts_type_to_collected_at_depth(&rt.type_annotation, depth + 1))
                    .unwrap_or(CollectedType::Void);
                Some(CollectedObjectField {
                    name,
                    collected_type: CollectedType::Function { params, param_names, return_type: Box::new(return_type) },
                    required: !sig.optional,
                    description: String::new(),
                })
            }
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(super) fn expression_to_string<'a>(&self, expr: &Expression<'a>) -> String {
        match expr {
            Expression::NumericLiteral(n) => n.value.to_string(),
            Expression::StringLiteral(s) => format!("\"{}\"", s.value.as_str()),
            Expression::BooleanLiteral(b) => b.value.to_string(),
            Expression::NullLiteral(_) => "null".to_owned(),
            _ => "unknown".to_owned(),
        }
    }

    // ─── TSTypeReference helpers ──────────────────────────────────────────────

    /// Extract the name part of a TSTypeReference (for checking FC, forwardRef, etc.)
    pub(super) fn extract_type_ref_name<'a>(&self, tr: &TSTypeReference<'a>) -> String {
        self.ts_type_name_str(&tr.type_name)
    }

    /// Extract the first type argument name from a type ref's type params.
    ///
    /// Used for `FC<ButtonProps>` → "ButtonProps".
    /// Handles `PropsWithChildren<P>` and `Readonly<P>` wrappers.
    pub(super) fn extract_props_arg<'a>(
        &mut self,
        type_params: &Option<OxcBox<'a, TSTypeParameterInstantiation<'a>>>,
    ) -> Option<(CompactString, Vec<CollectedType>)> {
        let tp = type_params.as_ref()?;
        let first = tp.params.first()?;
        self.extract_type_name_from_type(first)
    }

    /// Get the (name, type_args) of a TSType if it's a simple named reference.
    ///
    /// Unwraps single-layer wrappers like `PropsWithChildren<P>` and `Readonly<P>`.
    pub(super) fn extract_type_name_from_type<'a>(
        &mut self,
        ty: &TSType<'a>,
    ) -> Option<(CompactString, Vec<CollectedType>)> {
        match ty {
            TSType::TSTypeReference(tr) => {
                // `Required<P>`, `Readonly<P>` and `PropsWithChildren<P>` stay as written: the resolver applies them
                // (required flags, an added `children`), which unwrapping them here used to throw away.
                let name = self.extract_type_ref_name(tr);
                let args = self.extract_type_args(&tr.type_arguments);
                Some((name.into(), args))
            }
            TSType::TSParenthesizedType(p) => self.extract_type_name_from_type(&p.type_annotation),
            TSType::TSUnionType(u) => {
                let members: Vec<CollectedType> = u.types.iter().map(|t| self.ts_type_to_collected(t)).collect();
                let bare = format!("__anon_{}", self.data.type_aliases.len());
                let scoped = self.scoped_key(&bare);
                self.data
                    .type_aliases
                    .insert(scoped, CollectedTypeAlias::Union { members, file_path: self.file_path.clone() });
                Some((bare.into(), vec![]))
            }
            TSType::TSIntersectionType(i) => {
                let members: Vec<CollectedType> = i.types.iter().map(|t| self.ts_type_to_collected(t)).collect();
                let bare = format!("__anon_{}", self.data.type_aliases.len());
                let scoped = self.scoped_key(&bare);
                self.data
                    .type_aliases
                    .insert(scoped, CollectedTypeAlias::Intersection { members, file_path: self.file_path.clone() });
                Some((bare.into(), vec![]))
            }
            TSType::TSTypeLiteral(_) => {
                // Bare inline object type used directly as props, e.g. `FC<{ x: string }>`
                // or `forwardRef<Elem, { x: string }>`. Synthesize an anonymous passthrough
                // alias so the resolver's existing alias machinery can resolve it, instead
                // of silently dropping the whole component (the `_ => None` fallback below).
                let collected = self.ts_type_to_collected(ty);
                let bare = format!("__anon_{}", self.data.type_aliases.len());
                let scoped = self.scoped_key(&bare);
                self.data.type_aliases.insert(
                    scoped,
                    CollectedTypeAlias::Passthrough { target: collected, file_path: self.file_path.clone() },
                );
                Some((bare.into(), vec![]))
            }
            _ => None,
        }
    }

    // ─── TSInterfaceHeritage → ExtendsRef ────────────────────────────────────

    pub(super) fn collect_extends<'a>(&mut self, ext: &TSInterfaceHeritage<'a>) -> ExtendsRef {
        let name = self.ts_type_name_str(&ext.type_name);
        let type_args = self.extract_type_args(&ext.type_arguments);
        self.classify_extends(&name, type_args)
    }

    pub(super) fn expression_to_ident_name<'a>(&self, expr: &Expression<'a>) -> String {
        match expr {
            Expression::Identifier(id) => id.name.as_str().to_owned(),
            Expression::StaticMemberExpression(me) => {
                format!("{}.{}", self.expression_to_ident_name(&me.object), me.property.name.as_str())
            }
            _ => "unknown".to_owned(),
        }
    }

    // ─── Property Signature collection ───────────────────────────────────────

    pub(super) fn collect_property_signature<'a>(&mut self, sig: &TSSignature<'a>) -> Option<RawProp> {
        match sig {
            TSSignature::TSPropertySignature(ps) => {
                let name = ps.key.static_name()?.to_string();
                let collected_type = ps
                    .type_annotation
                    .as_ref()
                    .map(|ta| self.ts_type_to_collected(&ta.type_annotation))
                    .unwrap_or(CollectedType::Any);

                let (description, tags) = self.find_jsdoc_with_tags(ps.span.start);

                Some(RawProp {
                    name,
                    collected_type,
                    required: !ps.optional,
                    description,
                    tags,
                    span_start: ps.span.start,
                    span_end: ps.span.end,
                })
            }
            TSSignature::TSMethodSignature(ms) => {
                let name = ms.key.static_name()?.to_string();
                let (description, tags) = self.find_jsdoc_with_tags(ms.span.start);
                let params: Vec<CollectedType> = ms
                    .params
                    .items
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|ta| self.ts_type_to_collected(&ta.type_annotation))
                            .unwrap_or(CollectedType::Any)
                    })
                    .collect();
                let param_names: Vec<Option<CompactString>> =
                    ms.params.items.iter().map(|p| binding_pattern_name(&p.pattern)).collect();
                let return_type = ms
                    .return_type
                    .as_ref()
                    .map(|rt| self.ts_type_to_collected(&rt.type_annotation))
                    .unwrap_or(CollectedType::Void);
                Some(RawProp {
                    name,
                    collected_type: CollectedType::Function { params, param_names, return_type: Box::new(return_type) },
                    required: !ms.optional,
                    description,
                    tags,
                    span_start: ms.span.start,
                    span_end: ms.span.end,
                })
            }
            _ => None,
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

pub(super) fn is_pascal_case(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_uppercase())
}

/// A parameter's simple identifier name, if it has one (not a destructured pattern).
fn binding_pattern_name(pattern: &BindingPattern) -> Option<CompactString> {
    match pattern {
        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str().into()),
        _ => None,
    }
}

/// Get the declared name from a Declaration node.
pub(super) fn declaration_name<'a>(decl: &Declaration<'a>) -> Option<&'a str> {
    match decl {
        Declaration::VariableDeclaration(vd) => vd.declarations.first().and_then(|d| match &d.id {
            BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
            _ => None,
        }),
        Declaration::FunctionDeclaration(f) => f.id.as_ref().map(|id| id.name.as_str()),
        Declaration::ClassDeclaration(c) => c.id.as_ref().map(|id| id.name.as_str()),
        Declaration::TSTypeAliasDeclaration(ta) => Some(ta.id.name.as_str()),
        Declaration::TSInterfaceDeclaration(iface) => Some(iface.id.name.as_str()),
        Declaration::TSEnumDeclaration(e) => Some(e.id.name.as_str()),
        Declaration::TSNamespaceDeclaration(ns) => Some(ns.id.name.as_str()),
        Declaration::TSExternalModuleDeclaration(m) => Some(m.id.value.as_str()),
        _ => None,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// One-based line and column (in characters) of a byte offset, clamped to the end of the source.
fn line_and_column(source: &str, offset: u32) -> (u32, u32) {
    let mut end = (offset as usize).min(source.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let before = &source[..end];
    let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line as u32, column as u32)
}

#[cfg(test)]
mod tests;
