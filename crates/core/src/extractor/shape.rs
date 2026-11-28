//! The one reader of a call's callee and type arguments and of a class's base, in the shape OXC 0.158 produced.
//!
//! OXC 0.159 keeps a parenthesised instantiation, `(f<A>)(…)` or `extends (Base<P>)`, on the callee or base, where 0.158
//! moved its type arguments onto the call or the heritage. On an optional call, `(f<A>)?.()`, both keep it. Every
//! extractor and resolver read of these parts goes through [`call`] or [`base`]; the architecture test below fails on a
//! direct read anywhere else.

use oxc_ast::ast::{CallExpression, Class, Expression, TSTypeParameterInstantiation};

/// A call's callee and type arguments as OXC 0.158 shaped them.
#[derive(Clone, Copy)]
pub(super) struct CallShape<'b, 'a> {
    /// The function called.
    pub(super) function: &'b Expression<'a>,
    pub(super) type_args: Option<&'b TSTypeParameterInstantiation<'a>>,
}

pub(super) fn call<'a, 'b>(call: &'b CallExpression<'a>) -> CallShape<'b, 'a> {
    match &call.callee {
        Expression::TSInstantiationExpression(instantiation) if !call.optional && call.type_arguments.is_none() => {
            CallShape { function: &instantiation.expression, type_args: Some(&instantiation.type_arguments) }
        }
        callee => CallShape { function: callee, type_args: call.type_arguments.as_deref() },
    }
}

/// A class's base and its type arguments as OXC 0.158 shaped them: `extends (Base<P>)` is `Base` with `P`.
#[derive(Clone, Copy)]
pub(super) struct BaseShape<'b, 'a> {
    pub(super) base: &'b Expression<'a>,
    pub(super) type_args: Option<&'b TSTypeParameterInstantiation<'a>>,
}

pub(super) fn base<'a, 'b>(class: &'b Class<'a>) -> Option<BaseShape<'b, 'a>> {
    let heritage = class.heritage.as_ref()?;
    Some(match &heritage.expression {
        Expression::TSInstantiationExpression(instantiation) if heritage.type_arguments.is_none() => {
            BaseShape { base: &instantiation.expression, type_args: Some(&instantiation.type_arguments) }
        }
        base => BaseShape { base, type_args: heritage.type_arguments.as_deref() },
    })
}

#[cfg(test)]
mod tests {
    /// The reads the source scan allows, each by file and by a receiver it recognises: a bare identifier bound to
    /// something that is no call and no class (a type node, a field of our own, a span identity check).
    const ALLOWED: &[(&str, &str, &str)] = &[
        ("extractor/body_facts.rs", "call.callee", "a span identity: whether a child is the callee node"),
        ("extractor/body_facts.rs", "new.callee", "a span identity: whether a child is the `new` callee node"),
        ("extractor/wrapper.rs", "pending.callee", "our `WrapperCallee` field, not the AST"),
        ("extractor/alias.rs", "tr.type_arguments", "a `TSTypeReference`, a type"),
        ("extractor/evidence.rs", "r.type_arguments", "a `TSTypeReference`, a type"),
        ("extractor/mod.rs", "tr.type_arguments", "a `TSTypeReference`, a type"),
        ("extractor/mod.rs", "ext.type_arguments", "a `TSInterfaceHeritage`, a type"),
        ("extractor/written.rs", "reference.type_arguments", "a `TSTypeReference`, a type"),
        ("extractor/component.rs", "reference.type_arguments", "a `TSTypeReference`, a type"),
        ("extractor/component.rs", "tr.type_arguments", "a `TSTypeReference`, a type"),
    ];

    /// The fields only `call` and `base` read. A call has no `type_parameters`; the extractor's reads of that name are
    /// of function, class and signature declarations, so it is not scanned.
    const FIELDS: &[&str] = &["callee", "type_arguments", "heritage", "super_class", "super_type_arguments"];

    /// The AST types a destructuring pattern names those fields of.
    const AST_TYPES: &[&str] = &[
        "CallExpression",
        "NewExpression",
        "TaggedTemplateExpression",
        "TSInstantiationExpression",
        "Class",
        "ClassHeritage",
    ];

    /// Rust source as tokens, without comments, strings and character literals: identifiers and single punctuation.
    fn tokens(source: &str) -> Vec<String> {
        let chars: Vec<char> = source.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        let ident = |c: char| c.is_alphanumeric() || c == '_';
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            if c.is_whitespace() {
                i += 1;
            } else if c == '/' && next == Some('/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if c == '/' && next == Some('*') {
                let mut depth = 0;
                while i < chars.len() {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            } else if (c == 'r' || c == 'b') && matches!(next, Some('"' | '#'))
                || (c == 'b' && next == Some('r') && matches!(chars.get(i + 2), Some('"' | '#')))
            {
                // A raw or byte string: skip the prefix, count the hashes, find the matching close.
                while i < chars.len() && chars[i] != '"' && chars[i] != '#' {
                    i += 1;
                }
                let hashes = chars[i..].iter().take_while(|c| **c == '#').count();
                i += hashes;
                if chars.get(i) != Some(&'"') {
                    out.push("#".to_owned());
                    continue;
                }
                i += 1;
                let raw = c == 'r' || next == Some('r');
                while i < chars.len() {
                    if !raw && chars[i] == '\\' {
                        i += 2;
                    } else if chars[i] == '"'
                        && chars[i + 1..].iter().take(hashes).filter(|c| **c == '#').count() == hashes
                    {
                        i += 1 + hashes;
                        break;
                    } else {
                        i += 1;
                    }
                }
            } else if c == '"' {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += if chars[i] == '\\' { 2 } else { 1 };
                }
                i += 1;
            } else if c == '\'' {
                // A character literal (`'.'`, `'\n'`) or a lifetime (`'a`).
                if next == Some('\\') {
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    i += 1;
                    while i < chars.len() && ident(chars[i]) {
                        i += 1;
                    }
                }
            } else if ident(c) {
                let start = i;
                while i < chars.len() && ident(chars[i]) {
                    i += 1;
                }
                out.push(chars[start..i].iter().collect());
            } else {
                out.push(c.to_string());
                i += 1;
            }
        }
        out
    }

    /// Every read of a protected field: `receiver.field` with its receiver when the receiver is a bare identifier
    /// (`call.callee`), and `None` for any other receiver (`x()?.callee`, `(*call).callee`, `a.b.callee`) or for a
    /// destructuring of an AST type (`CallExpression { callee, .. }`), which the allow-list never matches.
    fn reads(source: &str) -> Vec<(Option<String>, String)> {
        let tokens = tokens(source);
        let is_ident = |t: &str| t.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
        let mut reads = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            if !FIELDS.contains(&token.as_str()) || index == 0 {
                continue;
            }
            let before = tokens[index - 1].as_str();
            if before == "." {
                let receiver = index.checked_sub(2).map(|r| tokens[r].as_str());
                let chained = index.checked_sub(3).is_some_and(|r| tokens[r] == "." || tokens[r] == "?");
                let receiver = receiver.filter(|r| is_ident(r) && !chained).map(str::to_owned);
                reads.push((receiver, token.clone()));
            } else if matches!(before, "{" | "," | "ref" | "mut") {
                // A field in a struct pattern or literal: find the brace that opens it.
                let mut depth = 0usize;
                let mut opener = None;
                for back in (0..index).rev() {
                    match tokens[back].as_str() {
                        "}" | ")" | "]" => depth += 1,
                        "(" | "[" if depth > 0 => depth -= 1,
                        "{" if depth > 0 => depth -= 1,
                        "{" => {
                            opener = Some(back);
                            break;
                        }
                        "(" | "[" | ";" => break,
                        _ => {}
                    }
                }
                if opener.and_then(|o| o.checked_sub(1)).is_some_and(|o| AST_TYPES.contains(&tokens[o].as_str())) {
                    reads.push((None, token.clone()));
                }
            }
        }
        reads
    }

    /// The reads of `source`, as `file` holds it, that the allow-list does not cover.
    fn failures(
        file: &str,
        source: &str,
        used: &mut std::collections::BTreeSet<(&'static str, &'static str)>,
    ) -> Vec<String> {
        reads(source)
            .into_iter()
            .filter_map(|(receiver, field)| {
                let read = receiver.map(|receiver| format!("{receiver}.{field}"));
                let allowed =
                    read.as_deref().and_then(|read| ALLOWED.iter().find(|(f, r, _)| *f == file && *r == read));
                match allowed {
                    Some((f, r, _)) => {
                        used.insert((*f, *r));
                        None
                    }
                    None => Some(format!("{file}: {}", read.unwrap_or_else(|| format!("<unrecognised>.{field}")))),
                }
            })
            .collect()
    }

    /// No extractor or resolver source reads a callee, a call's type arguments or a class base itself: each goes
    /// through `call` or `base`, which read OXC 0.159's parenthesised instantiation as 0.158 shaped it. A read with a
    /// receiver the scan does not recognise, or a destructuring of an AST type, always fails.
    #[test]
    fn every_callee_and_base_read_goes_through_the_shape_accessor() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut used = std::collections::BTreeSet::new();
        for dir in ["extractor", "resolver"] {
            for entry in std::fs::read_dir(src.join(dir)).expect("the sources").flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".rs") || name == "shape.rs" || name.contains("tests") {
                    continue;
                }
                let source = std::fs::read_to_string(entry.path()).expect("a source");
                found.extend(failures(&format!("{dir}/{name}"), &source, &mut used));
            }
        }
        assert!(found.is_empty(), "reads outside `shape`:\n{}", found.join("\n"));
        // Each exception must match a read in its own file, and none may repeat.
        let stale: Vec<_> = ALLOWED.iter().filter(|(file, read, _)| !used.contains(&(*file, *read))).collect();
        assert!(stale.is_empty(), "allow-list entries that match nothing in their own file: {stale:?}");
        let mut seen = std::collections::BTreeSet::new();
        let duplicates: Vec<_> = ALLOWED.iter().filter(|(file, read, _)| !seen.insert((*file, *read))).collect();
        assert!(duplicates.is_empty(), "allow-list entries listed twice: {duplicates:?}");
    }

    /// The scan itself: each bypass is flagged, and comments, strings, our own fields and allowed reads pass.
    #[test]
    fn the_shape_scan_flags_every_bypass() {
        let flagged = [
            "let c = expr.as_call_expression()?.callee;",
            "let span = shape_of().callee.span;",
            "let CallExpression { callee, .. } = call;",
            "if let Some(CallExpression { type_arguments, callee, .. }) = found {}",
            "let CallExpression { ref callee, .. } = *call;",
            "match e { Expression::CallExpression(c) => &c.callee, _ => other }",
            "let args = call\n    .\n    type_arguments\n    .as_deref();",
            "let callee = &(*call).callee;",
            "let base = class.heritage.as_ref();",
            "let base = &boxed.class.super_class;",
            "let t = items[0].type_arguments;",
            "let c = call.callee;",
        ];
        for snippet in flagged {
            let mut used = std::collections::BTreeSet::new();
            assert!(!failures("extractor/evidence.rs", snippet, &mut used).is_empty(), "not flagged: {snippet}");
        }
        let passing = [
            ("extractor/evidence.rs", "// call.callee in a comment\n/* and (*call).callee /* nested */ here */"),
            ("extractor/evidence.rs", "let s = \"call.callee\"; let r = r#\"x()?.callee\"#; let b = b\"a.callee\";"),
            ("extractor/evidence.rs", "fn f<'a>(c: &'a Call) -> char { let _ = shape::call(c).function; '.' }"),
            ("extractor/evidence.rs", "let callee = shape::call(call).function; callee.span()"),
            ("extractor/wrapper.rs", "if let Some(callee) = pending.callee { PendingWrapper { callee, name } }"),
            ("extractor/body_facts.rs", "AstKind::CallExpression(call) => call.callee.span() != child,"),
        ];
        for (file, snippet) in passing {
            let mut used = std::collections::BTreeSet::new();
            assert_eq!(failures(file, snippet, &mut used), Vec::<String>::new(), "{snippet}");
        }
    }
}
