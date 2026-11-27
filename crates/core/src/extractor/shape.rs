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
    /// No extractor or resolver source reads a callee, a call's type arguments or a class base itself: each goes
    /// through `call` or `base`, which read OXC 0.159's parenthesised instantiation as 0.158 shaped it. Each allowed
    /// read names a receiver that is no call and no class: a type node, a field of our own, or a span identity check.
    #[test]
    fn every_callee_and_base_read_goes_through_the_shape_accessor() {
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
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut used = std::collections::BTreeSet::new();
        for dir in ["extractor", "resolver"] {
            for entry in std::fs::read_dir(src.join(dir)).expect("the sources").flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".rs") || name == "shape.rs" || name.contains("tests") {
                    continue;
                }
                let file = format!("{dir}/{name}");
                let source = std::fs::read_to_string(entry.path()).expect("a source");
                let code: String = source
                    .lines()
                    .map(|line| line.split("//").next().unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\n");
                for read in field_reads(&code) {
                    if let Some(allowed) = ALLOWED.iter().find(|(f, r, _)| *f == file && *r == read) {
                        used.insert(*allowed);
                    } else {
                        found.push(format!("{file}: {read}"));
                    }
                }
            }
        }
        assert!(found.is_empty(), "reads outside `shape`:\n{}", found.join("\n"));
        let stale: Vec<_> = ALLOWED.iter().filter(|entry| !used.contains(*entry)).collect();
        assert!(stale.is_empty(), "allow-list entries that match nothing: {stale:?}");
    }

    /// Every `receiver.field` read of a field the accessor owns, the receiver being the identifier before the dot,
    /// across whitespace and line breaks (`call\n    .type_arguments`).
    fn field_reads(code: &str) -> Vec<String> {
        const FIELDS: &[&str] = &["callee", "type_arguments", "heritage", "super_class", "super_type_arguments"];
        let bytes = code.as_bytes();
        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let mut reads = Vec::new();
        for (dot, _) in code.match_indices('.') {
            let after = code[dot + 1..].trim_start();
            let start = code.len() - after.len();
            let end = start + after.bytes().take_while(|b| word(*b)).count();
            let field = &code[start..end];
            if !FIELDS.contains(&field) {
                continue;
            }
            let before = code[..dot].trim_end();
            let receiver_start = before.len() - before.bytes().rev().take_while(|b| word(*b)).count();
            let receiver = &before[receiver_start..];
            if !receiver.is_empty() && !bytes[receiver_start].is_ascii_digit() {
                reads.push(format!("{receiver}.{field}"));
            }
        }
        reads
    }
}
