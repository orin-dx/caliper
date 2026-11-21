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

use caliper_docgen_core::types::PropParent;
use rustc_hash::{FxHashMap, FxHashSet};
use tokio::sync::OnceCell;

use crate::asked::{BatchAnswers, Mine, Taken, Theirs};
use crate::convert::declared_in;
use crate::rpc::{Asking, CallError, CompactSymbol, JsDocTag, Rpc, SymbolInfo, SymbolRef, decode, flag};

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

/// What one snapshot's answers about declarations are kept as: they do not change inside it. A question another
/// conversion is asking is waited for, not asked again ([`BatchAnswers`]).
#[derive(Default)]
pub(crate) struct Memo {
    descriptions: BatchAnswers<String, String>,
    tags: BatchAnswers<String, BTreeMap<String, String>>,
    /// By the declaration handle.
    symbols: BatchAnswers<String, DeclaredSymbol>,
    /// A parent symbol's name when it is an interface, else `None` (a type literal, a class, a module).
    parents: BatchAnswers<CompactSymbol, Option<String>>,
    /// The program's files by the path a declaration names: the path itself, and lower-cased.
    files: OnceCell<FxHashMap<String, String>>,
}

/// The request a batch holds for one answer, so the answer is read in the shape its method returns, and kept.
enum Pending {
    Description(Mine<String>),
    Tags(Mine<BTreeMap<String, String>>),
    Symbol(Mine<DeclaredSymbol>),
}

/// What a prop's JSDoc is kept by: its declarations, where the checker reads it, or the symbol when it has none.
fn jsdoc_key(property: &SymbolInfo) -> String {
    if property.declarations.is_empty() {
        property.reference.text().to_string()
    } else {
        property.declarations.join("\n")
    }
}

/// The declarations, description and tags of every prop of `asked`, in its order: at most one round trip for what
/// this snapshot has not been asked yet, one for the parents it has not named yet, and one per snapshot for its files.
pub(crate) async fn declared(rpc: &Rpc, memo: &Memo, asked: &[Asked<'_>]) -> Result<Vec<Declared>, CallError> {
    // A round asks what nobody is asking and then waits for what another conversion is: it sends before it waits, so
    // no wait is for an answer that waits on one of ours. A question an asker gave up on is asked by the next round.
    loop {
        let mut requests: Vec<(&'static str, Asking<'_>)> = Vec::new();
        let mut pending: Vec<Pending> = Vec::new();
        let mut theirs: Vec<Theirs> = Vec::new();
        let (mut described, mut tagged, mut declaring) =
            (FxHashSet::default(), FxHashSet::default(), FxHashSet::default());
        for prop in asked {
            let property = prop.property;
            let key = jsdoc_key(property);
            if prop.description && !described.contains(&key) {
                match memo.descriptions.take(key.clone()) {
                    Taken::Kept => {}
                    Taken::Mine(mine) => {
                        described.insert(key.clone());
                        requests.push(("getDocumentationComment", Asking::Symbol { symbol: &property.reference }));
                        pending.push(Pending::Description(mine));
                    }
                    Taken::Theirs(other) => theirs.push(other),
                }
            }
            if prop.tags && !tagged.contains(&key) {
                match memo.tags.take(key.clone()) {
                    Taken::Kept => {}
                    Taken::Mine(mine) => {
                        tagged.insert(key.clone());
                        requests.push(("getJsDocTags", Asking::Symbol { symbol: &property.reference }));
                        pending.push(Pending::Tags(mine));
                    }
                    Taken::Theirs(other) => theirs.push(other),
                }
            }
            if is_own_member(property) {
                continue;
            }
            for declaration in &property.declarations {
                if declaring.contains(declaration) {
                    continue;
                }
                match memo.symbols.take(declaration.clone()) {
                    Taken::Kept => {}
                    Taken::Mine(mine) => {
                        declaring.insert(declaration);
                        requests.push(("getSymbolOfDeclarationForChecker", Asking::Location { location: declaration }));
                        pending.push(Pending::Symbol(mine));
                    }
                    Taken::Theirs(other) => theirs.push(other),
                }
            }
        }
        if !requests.is_empty() {
            let answers = rpc.batch(requests).await?;
            for (request, answer) in pending.into_iter().zip(answers) {
                match request {
                    Pending::Description(mine) => mine.answer(decode(&answer)?),
                    Pending::Tags(mine) => {
                        let answered: Option<Vec<JsDocTag>> = decode(&answer)?;
                        mine.answer(tags_of(answered.unwrap_or_default()));
                    }
                    Pending::Symbol(mine) => {
                        let symbol: Option<SymbolInfo> = decode(&answer)?;
                        mine.answer(symbol.map(|symbol| (symbol.parent, symbol.reference)));
                    }
                }
            }
        }
        if theirs.is_empty() {
            break;
        }
        for other in theirs {
            other.wait().await;
        }
    }

    // The symbol each declaration of each prop declares.
    let declares: Vec<Vec<DeclaredSymbol>> = asked
        .iter()
        .map(|prop| {
            let property = prop.property;
            if is_own_member(property) {
                return vec![Some((property.parent.clone(), property.reference.clone()))];
            }
            property.declarations.iter().map(|declaration| memo.symbols.peek(declaration).flatten()).collect()
        })
        .collect();
    let parents = parent_names(rpc, memo, &declares).await?;
    let files = if parents.values().any(Option::is_some) {
        Some(memo.files.get_or_try_init(|| program_files(rpc)).await?)
    } else {
        None
    };

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
                memo.descriptions.peek(&key).unwrap_or_default()
            } else {
                String::new()
            },
            tags: if prop.tags { memo.tags.peek(&key).unwrap_or_default() } else { BTreeMap::new() },
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
/// those not named before and not being named by another conversion.
async fn parent_names(
    rpc: &Rpc,
    memo: &Memo,
    declares: &[Vec<DeclaredSymbol>],
) -> Result<FxHashMap<CompactSymbol, Option<String>>, CallError> {
    let mut seen = FxHashSet::default();
    let mut named: Vec<(&CompactSymbol, &SymbolRef)> = Vec::new();
    for (parent, reference) in
        declares.iter().flatten().flatten().filter_map(|(parent, reference)| Some((parent.as_ref()?, reference)))
    {
        if seen.insert(parent) {
            named.push((parent, reference));
        }
    }
    // As in [`declared`]: ask what nobody is asking, then wait for what another conversion is.
    loop {
        let mut requests = Vec::new();
        let mut pending: Vec<Mine<Option<String>>> = Vec::new();
        let mut theirs: Vec<Theirs> = Vec::new();
        for (parent, symbol) in &named {
            match memo.parents.take((*parent).clone()) {
                Taken::Kept => {}
                Taken::Mine(mine) => {
                    requests.push(("getParentOfSymbolForChecker", Asking::Symbol { symbol }));
                    pending.push(mine);
                }
                Taken::Theirs(other) => theirs.push(other),
            }
        }
        if !requests.is_empty() {
            let answers = rpc.batch(requests).await?;
            for (mine, answer) in pending.into_iter().zip(answers) {
                let symbol: Option<SymbolInfo> = decode(&answer)?;
                mine.answer(
                    symbol.filter(|symbol| symbol.flags & flag::SYMBOL_INTERFACE != 0).map(|symbol| symbol.name),
                );
            }
        }
        if theirs.is_empty() {
            break;
        }
        for other in theirs {
            other.wait().await;
        }
    }
    Ok(named.into_iter().map(|(parent, _)| (parent.clone(), memo.parents.peek(parent).flatten())).collect())
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

/// Each source, by its name and the file as the checker keys it, as a prop's `parent` names it ([`declared`]): the file
/// on disk, or the checker's spelling where the program does not name it.
pub(crate) async fn as_parents(
    rpc: &Rpc,
    memo: &Memo,
    sources: Vec<(String, String)>,
) -> Result<Vec<PropParent>, CallError> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let files = memo.files.get_or_try_init(|| program_files(rpc)).await?;
    Ok(sources
        .into_iter()
        .map(|(name, file)| PropParent { name, file_name: files.get(&file).cloned().unwrap_or(file) })
        .collect())
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
    use serde_json::json;

    use super::*;

    fn tag(name: &str, text: Option<&str>) -> JsDocTag {
        JsDocTag { name: name.to_string(), text: text.map(str::to_string) }
    }

    fn symbol(declarations: &[&str], parent: Option<u64>) -> SymbolInfo {
        SymbolInfo {
            reference: json!({ "id": 7 }).into(),
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
        let mapped = SymbolInfo { reference: json!({ "id": 99 }).into(), ..symbol(&["8.173./p/base.ts"], None) };
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

    /// The props of `Omit<Base, "y">` (`x` and nothing else) from a real checker: a mapped type's property names no
    /// parent, so its one declaration is asked for the symbol it declares.
    fn with_mapped_props<T>(check: impl AsyncFnOnce(Rpc, String, Vec<SymbolInfo>) -> T) -> T {
        use crate::checker::Backend;

        let dir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let file = root.join("C.ts").to_string();
        let source =
            "interface Base {\n  /** The x. */\n  x: string;\n  y: number;\n}\nexport type Props = Omit<Base, 'y'>;\n";
        let checker = Backend::start(&root, 1).unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let answered = runtime.block_on(async {
            let deadline = std::time::Duration::from_secs(60);
            let client = checker.client();
            let options = Rpc::prepare(&client, deadline, None).await.unwrap();
            let rpc =
                Rpc::open_in_memory(client, deadline, &options, &[(file.clone(), source.to_string())]).await.unwrap();
            let module = rpc.source_file_symbol(&file).await.unwrap().unwrap();
            let exports = rpc.exports_of_module(&module.reference).await.unwrap();
            let alias = exports.iter().find(|export| export.name == "Props").unwrap();
            let props = rpc.declared_type_of_symbol(&alias.reference).await.unwrap();
            let properties = rpc.properties(props.id).await.unwrap();
            assert_eq!(properties.iter().map(|property| property.name.as_str()).collect::<Vec<_>>(), ["x"]);
            assert!(properties[0].parent.is_none() && properties[0].declarations.len() == 1, "{:?}", properties[0]);
            check(rpc, file, properties).await
        });
        runtime.block_on(checker.shutdown());
        answered
    }

    /// Two conversions on one lane ask about the same prop at once: the declaration's symbol, the description and the
    /// tags are each asked of the checker once, and both get the answer. On the base each asked.
    #[test]
    fn two_conversions_asking_about_one_prop_at_once_send_one_request() {
        with_mapped_props(async |rpc, file, properties| {
            let memo = Memo::default();
            let asked = [Asked { property: &properties[0], description: true, tags: true }];

            let (first, second) = tokio::join!(declared(&rpc, &memo, &asked), declared(&rpc, &memo, &asked));

            let (first, second) = (first.unwrap(), second.unwrap());
            let calls = rpc.calls();
            let sent = |method: &str| calls.get(method).copied();
            assert_eq!(sent("getSymbolOfDeclarationForChecker"), Some(1), "{calls:?}");
            assert_eq!(sent("getDocumentationComment"), Some(1), "{calls:?}");
            assert_eq!(sent("getJsDocTags"), Some(1), "{calls:?}");
            assert_eq!(sent("getParentOfSymbolForChecker"), Some(1), "{calls:?}");
            let base = PropParent { name: "Base".to_string(), file_name: file };
            for answer in [first, second] {
                let [only] = answer.as_slice() else { panic!("{answer:?}") };
                assert_eq!((only.description.as_str(), &only.parent), ("The x.", &Some(base.clone())));
                assert_eq!(only.declarations, std::slice::from_ref(&base));
            }
            // Asked again, it is kept.
            declared(&rpc, &memo, &asked).await.unwrap();
            assert_eq!(rpc.calls().get("getSymbolOfDeclarationForChecker"), Some(&1));
            rpc.release().await;
        });
    }

    /// A question the checker rejects is not kept, for the asker or for one that waited for it: each asks again, as
    /// each did on the base.
    #[test]
    fn a_rejected_question_is_asked_again_by_the_next_asker_and_by_one_that_waited() {
        with_mapped_props(async |rpc, _, properties| {
            let memo = Memo::default();
            let unknown = SymbolInfo { declarations: vec!["1.1.nowhere.ts".to_string()], ..properties[0].clone() };
            let asked = [Asked { property: &unknown, description: false, tags: false }];

            assert!(declared(&rpc, &memo, &asked).await.is_err());
            assert_eq!(rpc.calls().get("getSymbolOfDeclarationForChecker"), Some(&1));
            assert!(declared(&rpc, &memo, &asked).await.is_err());
            assert_eq!(rpc.calls().get("getSymbolOfDeclarationForChecker"), Some(&2));

            // Two at once: the second waits for the first, whose failure it does not take as its own answer.
            let (first, second) = tokio::join!(declared(&rpc, &memo, &asked), declared(&rpc, &memo, &asked));
            assert!(first.is_err() && second.is_err());
            assert_eq!(rpc.calls().get("getSymbolOfDeclarationForChecker"), Some(&4));
            rpc.release().await;
        });
    }
}
