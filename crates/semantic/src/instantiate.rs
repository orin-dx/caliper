//! Generic components, read through an instantiation the checker forms itself.
//!
//! The props parameter of `function Poly<E extends ElementType = "div">(props: PolyProps<E>)` is `PolyProps<E>` with `E`
//! open, which has two props where `<Poly />` has every attribute of a `div`. The checker's API has no request that
//! instantiates a signature, so the instantiation is written as source: one file, held in the checker's memory and never
//! on disk (`createSnapshot` with a `fileSystem` layer), declares for each such component
//!
//! ```ts
//! import type { "Poly" as CaliperComponent0 } from "/project/src/Poly.tsx"; export type CaliperProbe0 = Parameters<typeof CaliperComponent0<"div">>[0];
//! ```
//!
//! and the props are those of `CaliperProbe0`, read as any props type is. The file is added to a second snapshot of the
//! same program, made only when a requested component is generic, so a project without one sends no request more. A
//! line the checker reports an error on (a default it prints with a name the file cannot see) sends that component to
//! its structural result with the error as the reason, and a checker that does not load the file sends them all; a
//! result is never read from an instantiation that did not form.
//!
//! A printed default is text, and the probe file is another module than the component's: a name it prints may resolve
//! there to another symbol than it does in the component's module (the module's own `FormData` against the global
//! one), which forms without an error. So the line also declares each type argument as a type of its own and the
//! signature as `CaliperSignature0`, and a component is read only when each argument is, in the checker that holds the
//! probe file, the very type its signature declares as the default ([`arguments_keep_their_meaning`]). That is the one
//! place the pattern "the probe's type arguments resolve, in the probe file, to what they resolve to in the component's
//! module" is enforced; a default that does not pass sends the component to its structural result.

use std::sync::Arc;

use caliper_docgen_core::types::semantic::{PropInterner, SemanticComponent, SemanticOutcome};
use camino::Utf8Path;
use rustc_hash::FxHashMap;

use crate::component;
use crate::convert::Converter;
use crate::rpc::{CallError, FileDiagnostic, Rpc, SignatureInfo, SignatureKind, SymbolRef};
use crate::{Target, fallback_of};

/// The file the instantiations are declared in, in the project's root directory. It exists only in the checker's memory.
const PROBE_FILE: &str = "__caliper_instantiations__.ts";

/// A generic component to read through an instantiation: the name its module exports it under, and the type arguments.
#[derive(Debug, Clone)]
pub(crate) struct Instantiation {
    pub export: String,
    pub arguments: Vec<String>,
}

/// The type arguments of the instantiation a generic component gets when written without any (`<Poly />`): its first
/// type parameter's declared default, as the checker prints it. The checker fills in the parameters after it from their
/// own defaults, which may refer to the first. `None` when the component is not generic or its first type parameter
/// declares no default: then no instantiation is the one a story renders, and the props keep the open parameters.
pub(crate) async fn default_arguments(rpc: &Rpc, signature: &SignatureInfo) -> Result<Option<Vec<String>>, CallError> {
    if signature.type_parameters.is_empty() {
        return Ok(None);
    }
    // The ids a signature lists are not handles the checker knows until it has answered for them itself.
    let parameters = rpc.type_parameters(signature.id).await?;
    let Some(first) = parameters.first() else { return Ok(None) };
    let Some(default) = rpc.default_of_type_parameter(first.id).await? else { return Ok(None) };
    Ok(Some(vec![rpc.type_to_string(default.id).await?]))
}

/// Whether each type argument the probe file declares for component `index` is the type its signature declares as the
/// default of that parameter, in the checker that holds the probe file: the same type, not one of the same name.
async fn arguments_keep_their_meaning(
    layered: &Rpc,
    probes: &FxHashMap<String, SymbolRef>,
    index: usize,
    instantiation: &Instantiation,
) -> Result<bool, CallError> {
    let Some(signature_alias) = probes.get(&format!("CaliperSignature{index}")) else { return Ok(false) };
    let component = layered.declared_type_of_symbol(signature_alias).await?;
    let signatures = layered.signatures(component.id, SignatureKind::Call).await?;
    let [signature] = signatures.as_slice() else { return Ok(false) };
    let parameters = layered.type_parameters(signature.id).await?;
    for (position, parameter) in parameters.iter().enumerate().take(instantiation.arguments.len()) {
        let Some(default) = layered.default_of_type_parameter(parameter.id).await? else { return Ok(false) };
        let Some(argument_alias) = probes.get(&format!("CaliperArgument{index}_{position}")) else {
            return Ok(false);
        };
        if layered.declared_type_of_symbol(argument_alias).await?.id != default.id {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The line of the probe file that instantiates `instantiation`, the component exported by `file`: the instantiation's
/// props, the component's signature and each type argument, which [`arguments_keep_their_meaning`] compares. Each
/// component is one line, so an error the checker reports is the error of the line it starts on.
fn probe_line(index: usize, file: &Utf8Path, instantiation: &Instantiation) -> String {
    let quoted = |text: &str| serde_json::Value::String(text.to_string()).to_string();
    let declared: String = instantiation
        .arguments
        .iter()
        .enumerate()
        .map(|(position, argument)| format!(" export type CaliperArgument{index}_{position} = {argument};"))
        .collect();
    format!(
        "import type {{ {export} as CaliperComponent{index} }} from {file}; export type CaliperProbe{index} = \
         Parameters<typeof CaliperComponent{index}<{arguments}>>[0]; export type CaliperSignature{index} = typeof \
         CaliperComponent{index};{declared}",
        export = quoted(&instantiation.export),
        file = quoted(file.as_str()),
        arguments = instantiation.arguments.join(", "),
    )
}

/// The written form of an instantiation, for a reason: `Poly<"div">`.
fn written(target: &Target, instantiation: &Instantiation) -> String {
    format!("{}<{}>", target.display_name, instantiation.arguments.join(", "))
}

/// Answers each generic component from its instantiation, in a snapshot of `rpc`'s program over `root_files` with the
/// probe file added. The snapshot is released before this returns.
pub(crate) async fn answer(
    rpc: &Rpc,
    root_files: &[String],
    project_root: &Utf8Path,
    interner: &PropInterner,
    reverse: bool,
    mut pending: Vec<(Target, Instantiation)>,
) -> Result<SemanticOutcome, CallError> {
    let mut outcome = SemanticOutcome::default();
    // The file's text is part of the request, so it must not depend on the order the components were answered in.
    pending.sort_by(|(a, _), (b, _)| {
        (&a.file_path, &a.display_name, &a.key).cmp(&(&b.file_path, &b.display_name, &b.key))
    });
    let probe = project_root.join(PROBE_FILE);
    let text: Vec<String> = pending
        .iter()
        .enumerate()
        .map(|(index, (target, instantiation))| probe_line(index, &target.file_path, instantiation))
        .collect();
    let all_fall_back = |outcome: &mut SemanticOutcome, reason: &str| {
        outcome.fallbacks.extend(pending.iter().map(|(target, _)| fallback_of(target, reason)));
    };

    let layered = match rpc.layered(root_files, &[(probe.to_string(), text.join("\n"))]).await {
        Ok(layered) => Arc::new(layered),
        Err(CallError::Rejected(message)) => {
            all_fall_back(&mut outcome, &format!("the checker could not instantiate a generic component: {message}"));
            return Ok(outcome);
        }
        Err(fatal) => return Err(fatal),
    };
    let answered = answer_layered(&layered, probe.as_str(), interner, reverse, &pending).await;
    layered.release().await;
    match answered {
        Ok(answered) => Ok(answered),
        Err(CallError::Rejected(message)) => {
            all_fall_back(&mut outcome, &format!("the checker rejected a request: {message}"));
            Ok(outcome)
        }
        Err(fatal) => Err(fatal),
    }
}

async fn answer_layered(
    layered: &Arc<Rpc>,
    probe: &str,
    interner: &PropInterner,
    reverse: bool,
    pending: &[(Target, Instantiation)],
) -> Result<SemanticOutcome, CallError> {
    let mut outcome = SemanticOutcome::default();
    let Some(module) = layered.source_file_symbol(probe).await? else {
        let reason = "the checker did not load the file that instantiates generic components";
        outcome.fallbacks.extend(pending.iter().map(|(target, _)| fallback_of(target, reason)));
        return Ok(outcome);
    };
    let probes: FxHashMap<String, SymbolRef> = layered
        .exports_of_module(&module.reference)
        .await?
        .into_iter()
        .map(|symbol| (symbol.name, symbol.reference))
        .collect();
    let mut errors: FxHashMap<usize, String> = FxHashMap::default();
    // The checker's answer for a file also holds errors it found in other files while forming the instantiations; a
    // line number says nothing about those, so only the probe file's own errors are the errors of a probe line.
    let in_probe = |diagnostic: &FileDiagnostic| diagnostic.file_name.replace('\\', "/") == probe.replace('\\', "/");
    for diagnostic in layered.semantic_diagnostics(probe).await?.into_iter().filter(|d| d.is_error() && in_probe(d)) {
        errors.entry(diagnostic.start_position.line).or_insert_with(|| diagnostic.message());
    }

    let converter = Converter::new(layered.clone(), reverse);
    for (index, (target, instantiation)) in pending.iter().enumerate() {
        let read = match (errors.get(&index), probes.get(&format!("CaliperProbe{index}"))) {
            (Some(error), _) => {
                Err(format!("the instantiation {} could not be formed: {error}", written(target, instantiation)))
            }
            (None, None) => Err(format!("the instantiation {} was not declared", written(target, instantiation))),
            (None, Some(symbol)) => {
                let kept = arguments_keep_their_meaning(layered, &probes, index, instantiation).await;
                match kept {
                    Ok(false) => Err(format!(
                        "the instantiation {} could not be formed: a name in its type arguments means something \
                         else where the instantiation is written than in the component's module",
                        written(target, instantiation)
                    )),
                    Ok(true) => match read_probe(layered, &converter, interner, symbol, target).await {
                        Ok(read) => read,
                        Err(CallError::Rejected(message)) => Err(format!("the checker rejected a request: {message}")),
                        Err(fatal) => return Err(fatal),
                    },
                    Err(CallError::Rejected(message)) => Err(format!("the checker rejected a request: {message}")),
                    Err(fatal) => return Err(fatal),
                }
            }
        };
        match read {
            Ok(component::Props { props, composes }) => {
                outcome.components.push(SemanticComponent { key: target.key.clone(), props, composes });
            }
            Err(reason) => outcome.fallbacks.push(fallback_of(target, &reason)),
        }
    }
    Ok(outcome)
}

async fn read_probe(
    layered: &Rpc,
    converter: &Converter,
    interner: &PropInterner,
    symbol: &SymbolRef,
    target: &Target,
) -> Result<Result<component::Props, String>, CallError> {
    let props_type = layered.declared_type_of_symbol(symbol).await?;
    component::props_of_type(layered, converter, interner, &props_type, target).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_line_imports_the_export_by_its_quoted_name_and_instantiates_it() {
        let instantiation = Instantiation { export: "default".to_string(), arguments: vec!["\"div\"".to_string()] };

        let line = probe_line(3, Utf8Path::new("/p/src/Poly.tsx"), &instantiation);

        assert_eq!(
            line,
            "import type { \"default\" as CaliperComponent3 } from \"/p/src/Poly.tsx\"; export type CaliperProbe3 = \
             Parameters<typeof CaliperComponent3<\"div\">>[0]; export type CaliperSignature3 = typeof \
             CaliperComponent3; export type CaliperArgument3_0 = \"div\";"
        );
    }

    #[test]
    fn a_path_with_quotes_or_backslashes_stays_one_string() {
        let instantiation = Instantiation { export: "Poly".to_string(), arguments: vec!["string".to_string()] };

        let line = probe_line(0, Utf8Path::new("C:\\p\\a\"b.tsx"), &instantiation);

        assert!(line.contains(r#"from "C:\\p\\a\"b.tsx";"#), "{line}");
    }

    /// ADR 0026's S5 gate: `Poly<"div">` has at least 260 props, and `Anchor<"a">`, the same props at another default,
    /// others. An instantiation is always at the defaults, so a second element is a second component.
    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]
    fn a_polymorphic_component_has_the_props_of_the_element_it_is_instantiated_with() {
        let root = camino::Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic");
        let file = root.join("Poly.tsx");
        let target = |name: &str| Target {
            key: name.to_string(),
            file_path: file.clone(),
            display_name: name.to_string(),
            exports: vec![caliper_docgen_core::types::ComponentExport::named("Poly")],
            described: Default::default(),
            react_wanted: None,
        };
        let at = |export: &str, element: &str| Instantiation {
            export: export.to_string(),
            arguments: vec![format!("\"{element}\"")],
        };
        let timeout = std::time::Duration::from_secs(60);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

        let outcome = runtime.block_on(async {
            let checker = crate::checker::Backend::start(&root, 1).unwrap();
            let tsconfig = root.join("tsconfig.json");
            let files = [file.to_string()];
            let rpc = Rpc::open_programs(checker.client(), timeout, Some(tsconfig.as_str()), &files, false, 1)
                .await
                .unwrap()
                .remove(0);
            let pending = vec![(target("Poly<div>"), at("Poly", "div")), (target("Poly<a>"), at("Anchor", "a"))];
            let outcome = answer(&rpc, &files, &root, &PropInterner::default(), false, pending).await.unwrap();
            rpc.release().await;
            checker.shutdown().await;
            outcome
        });

        assert_eq!(outcome.fallbacks, []);
        let names = |display: &str| -> Vec<String> {
            let component = outcome.components.iter().find(|c| c.key == display).expect("answered");
            component.props.iter().map(|prop| prop.name.clone()).collect()
        };
        let (div, a) = (names("Poly<div>"), names("Poly<a>"));
        assert!(div.len() >= 260, "Poly<\"div\"> has {} props", div.len());
        assert_ne!(div, a);
        assert!(a.contains(&"href".to_string()) && !div.contains(&"href".to_string()));
        eprintln!("Poly<\"div\">: {} props, Poly<\"a\">: {} props", div.len(), a.len());
    }
}
