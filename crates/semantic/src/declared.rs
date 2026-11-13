//! Where each of a component's props is declared and what its JSDoc says, from the checker: the interface each of its
//! declarations sits in (`parent`, `declarations`), its description and its tags.
//!
//! A property symbol names the symbol it is a member of (`parent`); one a mapped type or an intersection made
//! (`Omit<Base, "x">`, `A & B`) names none, and its declarations are asked for the symbols they declare, whose parent
//! is the interface that wrote them. A declaration names its file the way the checker keys files, lower-cased where file
//! names are not case-sensitive; the file is reported as the program read it (`getSourceFileNames`), so
//! `parent.fileName` is the path on disk, as the structural pass reports it.
//!
//! Every answer is kept for the snapshot, keyed by what it depends on: a declaration's symbol by the declaration, a
//! parent's name by the parent, and a prop's JSDoc by its declarations, which is where the checker reads it, so the
//! props a library's components share (an instantiation or an `Omit` makes a new symbol for each) are asked about once.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use caliper_docgen_core::types::PropParent;
use rustc_hash::FxHashMap;
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::convert::declared_in;
use crate::rpc::{CallError, CompactSymbol, JsDocTag, Rpc, SymbolInfo, SymbolRef, decode, flag};

/// One prop to look up, and which of its JSDoc to ask for.
pub(crate) struct Asked<'a> {
    pub(crate) property: &'a SymbolInfo,
    /// Ask for its description: the structural pass has none for it.
    pub(crate) description: bool,
    /// Ask for its tags.
    pub(crate) tags: bool,
}

/// What the checker says about where one prop is declared.
#[derive(Debug, Default)]
pub(crate) struct Declared {
    /// Empty when not asked for, or when it has none.
    pub(crate) description: String,
    /// Empty when not asked for, or when it has none.
    pub(crate) tags: BTreeMap<String, String>,
    pub(crate) parent: Option<PropParent>,
    pub(crate) declarations: Vec<PropParent>,
}

/// The symbol a declaration declares, by the parent it names and the reference that asks about it.
type DeclaredSymbol = Option<(Option<CompactSymbol>, SymbolRef)>;

/// What one snapshot's answers about declarations are kept as: they do not change inside it.
#[derive(Debug, Default)]
pub(crate) struct Memo {
    descriptions: Mutex<FxHashMap<String, String>>,
    tags: Mutex<FxHashMap<String, BTreeMap<String, String>>>,
    /// By the declaration handle.
    symbols: Mutex<FxHashMap<String, DeclaredSymbol>>,
    /// A parent symbol's name when it is an interface, else `None` (a type literal, a class, a module).
    parents: Mutex<FxHashMap<CompactSymbol, Option<String>>>,
    /// The program's files by the path a declaration names: the path itself, and lower-cased.
    files: OnceCell<FxHashMap<String, String>>,
}

/// The request a batch holds for one answer, so the answer is read in the shape its method returns, and kept by key.
enum Pending {
    Description(String),
    Tags(String),
    Symbol(String),
}

/// What a prop's JSDoc is kept by: its declarations, where the checker reads it, or the symbol when it has none.
fn jsdoc_key(property: &SymbolInfo) -> String {
    if property.declarations.is_empty() { property.reference.text().to_string() } else { property.declarations.join("\n") }
}

/// The declarations, description and tags of every prop of `asked`, in its order: at most one round trip for what
/// this snapshot has not been asked yet, one for the parents it has not named yet, and one per snapshot for its files.
pub(crate) async fn declared(rpc: &Rpc, memo: &Memo, asked: &[Asked<'_>]) -> Result<Vec<Declared>, CallError> {
    let mut requests: Vec<(&str, Value)> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    {
        let descriptions = memo.descriptions.lock().unwrap_or_else(PoisonError::into_inner);
        let tags = memo.tags.lock().unwrap_or_else(PoisonError::into_inner);
        let symbols = memo.symbols.lock().unwrap_or_else(PoisonError::into_inner);
        let mut queued = rustc_hash::FxHashSet::default();
        for prop in asked {
            let property = prop.property;
            let key = jsdoc_key(property);
            if prop.description && !descriptions.contains_key(&key) && queued.insert(("d", key.clone())) {
                requests.push(("getDocumentationComment", json!({ "symbol": property.reference })));
                pending.push(Pending::Description(key.clone()));
            }
            if prop.tags && !tags.contains_key(&key) && queued.insert(("t", key.clone())) {
                requests.push(("getJsDocTags", json!({ "symbol": property.reference })));
                pending.push(Pending::Tags(key));
            }
            if is_own_member(property) {
                continue;
            }
            for declaration in &property.declarations {
                if !symbols.contains_key(declaration) && queued.insert(("s", declaration.clone())) {
                    requests.push(("getSymbolOfDeclarationForChecker", json!({ "location": declaration })));
                    pending.push(Pending::Symbol(declaration.clone()));
                }
            }
        }
    }
    if !requests.is_empty() {
        let answers = rpc.batch(requests).await?;
        let mut descriptions = memo.descriptions.lock().unwrap_or_else(PoisonError::into_inner);
        let mut tags = memo.tags.lock().unwrap_or_else(PoisonError::into_inner);
        let mut symbols = memo.symbols.lock().unwrap_or_else(PoisonError::into_inner);
        for (request, answer) in pending.into_iter().zip(answers) {
            match request {
                Pending::Description(key) => {
                    descriptions.insert(key, decode(&answer)?);
                }
                Pending::Tags(key) => {
                    let answered: Option<Vec<JsDocTag>> = decode(&answer)?;
                    tags.insert(key, tags_of(answered.unwrap_or_default()));
                }
                Pending::Symbol(declaration) => {
                    let symbol: Option<SymbolInfo> = decode(&answer)?;
                    symbols.insert(declaration, symbol.map(|symbol| (symbol.parent, symbol.reference)));
                }
            }
        }
    }

    // The symbol each declaration of each prop declares.
    let declares: Vec<Vec<DeclaredSymbol>> = {
        let symbols = memo.symbols.lock().unwrap_or_else(PoisonError::into_inner);
        asked
            .iter()
            .map(|prop| {
                let property = prop.property;
                if is_own_member(property) {
                    return vec![Some((property.parent.clone(), property.reference.clone()))];
                }
                property.declarations.iter().map(|declaration| symbols.get(declaration).cloned().flatten()).collect()
            })
            .collect()
    };
    let parents = parent_names(rpc, memo, &declares).await?;
    let files = if parents.values().any(Option::is_some) {
        Some(memo.files.get_or_try_init(|| program_files(rpc)).await?)
    } else {
        None
    };

    let descriptions = memo.descriptions.lock().unwrap_or_else(PoisonError::into_inner);
    let tags = memo.tags.lock().unwrap_or_else(PoisonError::into_inner);
    let mut found = Vec::with_capacity(asked.len());
    for (prop, slots) in asked.iter().zip(&declares) {
        let key = jsdoc_key(prop.property);
        let mut interfaces = prop.property.declarations.iter().zip(slots).map(|(declaration, slot)| {
            let parent = slot.as_ref()?.0.as_ref()?;
            let name = parents.get(parent).cloned().flatten()?;
            let file = declared_in(declaration);
            let file_name = files.and_then(|files| files.get(file)).cloned().unwrap_or_else(|| file.to_string());
            Some(PropParent { name, file_name })
        });
        // react-docgen-typescript's `parent`: the first declaration's, and only when an interface holds it.
        let parent = interfaces.next().flatten();
        let declarations = parent.iter().cloned().chain(interfaces.flatten()).collect();
        found.push(Declared {
            description: if prop.description {
                descriptions.get(&key).cloned().unwrap_or_default()
            } else {
                String::new()
            },
            tags: if prop.tags { tags.get(&key).cloned().unwrap_or_default() } else { BTreeMap::new() },
            parent,
            declarations,
        });
    }
    Ok(found)
}

/// Whether the property is the symbol its one declaration declares, a member of the parent it names: then that
/// declaration need not be asked for its symbol.
fn is_own_member(property: &SymbolInfo) -> bool {
    property.parent.is_some() && property.declarations.len() == 1
}

/// The name of each parent `declares` names, when it is an interface: asked once per snapshot, in one round trip for
/// those not named before.
async fn parent_names(
    rpc: &Rpc,
    memo: &Memo,
    declares: &[Vec<DeclaredSymbol>],
) -> Result<FxHashMap<CompactSymbol, Option<String>>, CallError> {
    let mut known: FxHashMap<CompactSymbol, Option<String>> = FxHashMap::default();
    let mut unseen: Vec<(CompactSymbol, &SymbolRef)> = Vec::new();
    {
        let seen = memo.parents.lock().unwrap_or_else(PoisonError::into_inner);
        let named = declares.iter().flatten().flatten();
        for (parent, reference) in named.filter_map(|(parent, reference)| Some((parent.as_ref()?, reference))) {
            if known.contains_key(parent) {
                continue;
            }
            match seen.get(parent) {
                Some(name) => {
                    known.insert(parent.clone(), name.clone());
                }
                None => {
                    known.insert(parent.clone(), None);
                    unseen.push((parent.clone(), reference));
                }
            }
        }
    }
    if unseen.is_empty() {
        return Ok(known);
    }
    let requests =
        unseen.iter().map(|(_, reference)| ("getParentOfSymbolForChecker", json!({ "symbol": reference }))).collect();
    let answers = rpc.batch(requests).await?;
    let mut seen = memo.parents.lock().unwrap_or_else(PoisonError::into_inner);
    for ((parent, _), answer) in unseen.into_iter().zip(answers) {
        let symbol: Option<SymbolInfo> = decode(&answer)?;
        let name = symbol.filter(|symbol| symbol.flags & flag::SYMBOL_INTERFACE != 0).map(|symbol| symbol.name);
        seen.insert(parent.clone(), name.clone());
        known.insert(parent, name);
    }
    Ok(known)
}

/// The program's files, by their own path and by its lower-cased form, the first file of a lower-cased form winning.
/// A declaration in a file the list does not name keeps the checker's spelling.
async fn program_files(rpc: &Rpc) -> Result<FxHashMap<String, String>, CallError> {
    let names = rpc.source_file_names().await?;
    let mut files = FxHashMap::default();
    for name in &names {
        files.insert(name.clone(), name.clone());
    }
    for name in names {
        files.entry(name.to_lowercase()).or_insert(name);
    }
    Ok(files)
}

/// A prop's tags as the structural pass keys them: by name, the last of a repeated tag winning, its text trimmed, and a
/// `@param` by `param:<parameter>` with the text after the parameter's name, which the checker renders as the text's
/// first word.
fn tags_of(tags: Vec<JsDocTag>) -> BTreeMap<String, String> {
    tags.into_iter()
        .map(|tag| {
            let text = tag.text.unwrap_or_default();
            let text = text.trim();
            match (tag.name.as_str(), text.split_once(char::is_whitespace)) {
                ("param", Some((parameter, rest))) => (format!("param:{parameter}"), rest.trim().to_string()),
                ("param", None) if !text.is_empty() => (format!("param:{text}"), String::new()),
                _ => (tag.name, text.to_string()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str, text: Option<&str>) -> JsDocTag {
        JsDocTag { name: name.to_string(), text: text.map(str::to_string) }
    }

    fn symbol(declarations: &[&str], parent: Option<u64>) -> SymbolInfo {
        SymbolInfo {
            reference: SymbolRef::of(r#"{"id":7}"#),
            name: "x".to_string(),
            flags: 4,
            declarations: declarations.iter().map(|declaration| declaration.to_string()).collect(),
            parent: parent.map(|id| CompactSymbol { id, file: None }),
        }
    }

    #[test]
    fn a_tag_is_keyed_by_name_with_its_text_trimmed_and_a_repeated_tag_keeps_the_last() {
        let tags = tags_of(vec![
            tag("deprecated", None),
            tag("see", Some(" https://a.example ")),
            tag("since", Some("2.0")),
            tag("see", Some("https://b.example")),
        ]);

        assert_eq!(
            tags,
            BTreeMap::from([
                ("deprecated".to_string(), String::new()),
                ("see".to_string(), "https://b.example".to_string()),
                ("since".to_string(), "2.0".to_string()),
            ])
        );
    }

    #[test]
    fn a_param_tag_is_keyed_by_its_parameter_as_the_structural_pass_keys_it() {
        let tags = tags_of(vec![tag("param", Some("target - The target element.")), tag("param", Some("event"))]);

        assert_eq!(
            tags,
            BTreeMap::from([
                ("param:event".to_string(), String::new()),
                ("param:target".to_string(), "- The target element.".to_string()),
            ])
        );
    }

    #[test]
    fn jsdoc_is_kept_by_the_declarations_so_two_symbols_of_one_declaration_share_it() {
        let declared = symbol(&["8.173./p/base.ts"], Some(3));
        let mapped = SymbolInfo { reference: SymbolRef::of(r#"{"id":99}"#), ..symbol(&["8.173./p/base.ts"], None) };
        let synthetic = symbol(&[], None);

        assert_eq!(jsdoc_key(&declared), jsdoc_key(&mapped));
        assert_eq!(jsdoc_key(&synthetic), r#"{"id":7}"#);
    }

    #[test]
    fn only_a_member_with_its_parent_and_one_declaration_skips_asking_for_the_declared_symbol() {
        assert!(is_own_member(&symbol(&["8.173./p/base.ts"], Some(3))));
        assert!(!is_own_member(&symbol(&["8.173./p/base.ts"], None)));
        assert!(!is_own_member(&symbol(&["8.173./p/a.ts", "9.173./p/b.ts"], Some(3))));
    }
}
