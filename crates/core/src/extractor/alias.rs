//! Type alias classification: Omit, Pick, Partial, Required, Union, Intersection, Passthrough.

use oxc_ast::ast::*;
use oxc_span::GetSpan;

use crate::types::{CollectedType, CollectedTypeAlias, DiagnosticCode};

use super::SourceDataCollector;

impl<'src> SourceDataCollector<'src> {
    // ─── TypeAlias classification ─────────────────────────────────────────────

    pub(super) fn classify_type_alias<'a>(&mut self, _name: &str, ty: &TSType<'a>) -> Option<CollectedTypeAlias> {
        let fp = self.file_path.clone();
        match ty {
            TSType::TSTypeReference(tr) => {
                let ref_name = self.extract_type_ref_name(tr);
                match ref_name.as_str() {
                    "Omit" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Omit<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        if tp.params.len() < 2 {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Omit<> with fewer than 2 type arguments"),
                                tp.span,
                            );
                            return None;
                        }
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(&tp.params[0]) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Omit<>'s base type argument is not a recognizable type reference"),
                                tp.params[0].span(),
                            );
                            return None;
                        };
                        let base = CollectedType::Named { name: base_name, args: base_args };
                        let (omitted_keys, omitted_keys_of) = self.collect_omit_keys(&tp.params[1]);
                        Some(CollectedTypeAlias::Omit { base, omitted_keys, omitted_keys_of, file_path: fp })
                    }
                    "Pick" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Pick<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        if tp.params.len() < 2 {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Pick<> with fewer than 2 type arguments"),
                                tp.span,
                            );
                            return None;
                        }
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(&tp.params[0]) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Pick<>'s base type argument is not a recognizable type reference"),
                                tp.params[0].span(),
                            );
                            return None;
                        };
                        let base = CollectedType::Named { name: base_name, args: base_args };
                        let picked_keys = self.collect_string_union_keys(&tp.params[1]);
                        Some(CollectedTypeAlias::Pick { base, picked_keys, file_path: fp })
                    }
                    "Partial" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Partial<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        let Some(first) = tp.params.first() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Partial<> with an empty type argument list"),
                                tp.span,
                            );
                            return None;
                        };
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(first) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Partial<>'s type argument is not a recognizable type reference"),
                                first.span(),
                            );
                            return None;
                        };
                        let base = CollectedType::Named { name: base_name, args: base_args };
                        Some(CollectedTypeAlias::Partial { base, file_path: fp })
                    }
                    "Required" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Required<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        let Some(first) = tp.params.first() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Required<> with an empty type argument list"),
                                tp.span,
                            );
                            return None;
                        };
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(first) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Required<>'s type argument is not a recognizable type reference"),
                                first.span(),
                            );
                            return None;
                        };
                        let base = CollectedType::Named { name: base_name, args: base_args };
                        Some(CollectedTypeAlias::Required { base, file_path: fp })
                    }
                    "Readonly" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Readonly<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        let Some(first) = tp.params.first() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Readonly<> with an empty type argument list"),
                                tp.span,
                            );
                            return None;
                        };
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(first) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Readonly<>'s type argument is not a recognizable type reference"),
                                first.span(),
                            );
                            return None;
                        };
                        let target = CollectedType::Named { name: base_name, args: base_args };
                        Some(CollectedTypeAlias::Passthrough { target, file_path: fp })
                    }
                    _ => {
                        // Simple passthrough: `type Foo<T, U> = SomeOtherType<T, U>`. Args
                        // are kept structured (not stringified) so that when `SomeOtherType`
                        // is itself a generic alias, the resolver's call-site substitution
                        // can walk into them — see resolver/substitute.rs. Stringifying here
                        // (the old behavior) collapsed nested generics like `Bar<T>` into an
                        // opaque display string before substitution ever ran.
                        let args: Vec<CollectedType> = tr
                            .type_arguments
                            .as_ref()
                            .map(|ta| ta.params.iter().map(|p| self.ts_type_to_collected(p)).collect())
                            .unwrap_or_default();
                        let target = CollectedType::Named { name: ref_name.into(), args };
                        Some(CollectedTypeAlias::Passthrough { target, file_path: fp })
                    }
                }
            }
            TSType::TSUnionType(u) => {
                // `undefined` is dropped from the literal set, as `resolve_union` does —
                // optionality lives in `required`. `null` is a real value a string-only
                // `LiteralUnion` can't hold, so it keeps the alias a plain `Union`.
                let all_string_literals = u.types.iter().all(|t| match t {
                    TSType::TSLiteralType(lit) => {
                        matches!(lit.literal, TSLiteral::StringLiteral(_))
                    }
                    TSType::TSUndefinedKeyword(_) => true,
                    _ => false,
                });

                let members: Vec<CollectedType> = u.types.iter().map(|t| self.ts_type_to_collected(t)).collect();

                if all_string_literals {
                    // Bare values, not `to_raw_string()`: every serializer quotes
                    // `LiteralUnion` members itself, so a pre-quoted member renders as `""sm""`.
                    let values: Vec<String> = members
                        .iter()
                        .filter_map(
                            |m| if let CollectedType::StringLiteral(s) = m { Some(s.to_string()) } else { None },
                        )
                        .collect();
                    if !values.is_empty() {
                        return Some(CollectedTypeAlias::LiteralUnion { members: values, file_path: fp });
                    }
                }
                Some(CollectedTypeAlias::Union { members, file_path: fp })
            }
            TSType::TSIntersectionType(i) => {
                let members = i.types.iter().map(|t| self.ts_type_to_collected(t)).collect();
                Some(CollectedTypeAlias::Intersection { members, file_path: fp })
            }
            TSType::TSParenthesizedType(p) => self.classify_type_alias(_name, &p.type_annotation),
            // Inline object type: `type Foo = { a: string }`. Previously fell through
            // to `_ => None` and silently vanished from data.type_aliases with no
            // diagnostic — anything referencing `Foo` would then resolve as unknown.
            TSType::TSTypeLiteral(_) => {
                Some(CollectedTypeAlias::Passthrough { target: self.ts_type_to_collected(ty), file_path: fp })
            }
            // Bare function type: `type Handler<T> = (arg: T) => void`. Same
            // silent-vanishing bug as TSTypeLiteral above — real-world callback type
            // aliases (react-day-picker's `OnSelectHandler<T>`) use this shape.
            TSType::TSFunctionType(_) => {
                Some(CollectedTypeAlias::Passthrough { target: self.ts_type_to_collected(ty), file_path: fp })
            }
            // Everything else `ts_type_to_collected` already knows how to represent
            // structurally (arrays, tuples, indexed access, conditional/mapped
            // types, …) — e.g. `type API_KeyCollection = string[]` (Storybook's real
            // pattern). Same silent-vanishing bug as the two arms above, generalized:
            // a dedicated arm above always wins for shapes needing special alias
            // semantics (Omit's key-splitting, discriminated-union detection, …); this
            // catch-all only ever runs for shapes with no such semantics, where a
            // transparent Passthrough is exactly correct.
            _ => Some(CollectedTypeAlias::Passthrough { target: self.ts_type_to_collected(ty), file_path: fp }),
        }
    }

    /// Collect the string literal keys from a type like `'key1' | 'key2'`.
    pub(super) fn collect_string_union_keys<'a>(&self, ty: &TSType<'a>) -> Vec<String> {
        match ty {
            TSType::TSLiteralType(lit) => match &lit.literal {
                TSLiteral::StringLiteral(s) => vec![s.value.as_str().to_owned()],
                _ => vec![],
            },
            TSType::TSUnionType(u) => u.types.iter().flat_map(|t| self.collect_string_union_keys(t)).collect(),
            _ => vec![],
        }
    }

    /// Classify `Omit<_, Keys>`'s second type argument: a literal key union
    /// (`'a' | 'b'`), or `keyof SomeType` — in the latter case the key set can't
    /// be known until `SomeType` is resolved, so the operand is captured
    /// structurally for the resolver to expand later (see
    /// `CollectedTypeAlias::Omit::omitted_keys_of`).
    pub(super) fn collect_omit_keys<'a>(&mut self, ty: &TSType<'a>) -> (Vec<String>, Option<Box<CollectedType>>) {
        match self.ts_type_to_collected(ty) {
            CollectedType::KeyOf(inner) => (vec![], Some(inner)),
            other => (other.as_string_union_keys(), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use crate::extractor::parse_file;
    use crate::types::DiagnosticCode;

    #[test]
    fn malformed_omit_missing_second_arg_records_skipped_candidate() {
        let source = r#"
            interface Foo { a: string; }
            type BadOmit = Omit<Foo>;
        "#;
        let path = Utf8Path::new("/test/bad-omit.ts");
        let data = parse_file(path, source);

        assert!(
            !data.type_aliases.contains_key("/test/bad-omit.ts:BadOmit"),
            "malformed Omit should still not produce a usable alias"
        );
        assert!(
            data.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate),
            "expected a SkippedCandidate diagnostic, got: {:?}",
            data.diagnostics
        );
    }
}
