#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! The backend against the `tsrs` it links: what it reads as an error type, and the lib it embeds.

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::semantic::{SemanticOutcome, merge_semantic};
use caliper_docgen_core::types::{ObjectField, PropParent, PropType};
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

/// The checker gives an unresolved import or an undeclared name the flag of `any` as well; only its `intrinsicName`
/// (`"error"`, where a written `any` says `"any"`) tells them apart. An error type is not `any`: the prop says it could
/// not be resolved.
#[test]
fn tsrs_marks_an_unresolved_name_degraded_and_a_written_any_not() {
    let dir = tempfile::Builder::new().prefix("TsrsReal").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixture = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/errors");
    for name in ["Errors.tsx", "tsconfig.json"] {
        std::fs::copy(fixture.join(name), root.join(name)).unwrap();
    }
    let mut output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let outcome = resolve(SemanticRequest::new(root, targets_of(&output)));
    assert!(outcome.diagnostics.is_empty() && outcome.fallbacks.is_empty(), "{outcome:?}");
    merge_semantic(&mut output, outcome);

    let props = &output.components["Errors"].props;
    assert_eq!(props["explicitAny"].prop_type, PropType::Any);
    assert!(props["explicitAny"].degraded_by.is_empty());
    assert_eq!(props["label"].prop_type, PropType::String);
    for name in ["fromImport", "undeclared"] {
        assert_eq!(props[name].degraded_by, ["unresolvableImport"], "{name}: {:?}", props[name].prop_type);
    }
}

/// The built-in checker reads TypeScript's lib from the files it embeds, named `bundled:///libs/lib.*.d.ts`. Those are
/// still the lib: an interface it declares keeps its name, an alias it
/// declares (`Partial`) is expanded, and a prop it declares names the lib file as its parent's.
#[test]
fn the_lib_the_built_in_checker_embeds_is_read_as_the_lib() {
    let dir = tempfile::Builder::new().prefix("Lib").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    std::fs::write(
        root.join("Lib.tsx"),
        "export interface LibProps extends Pick<Event, \"type\"> {\n  when: Date;\n  partial: Partial<{ x: number }>;\n}\n\
         export function Lib(props: LibProps) { return null; }\n",
    )
    .unwrap();
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });

    let outcome = resolve(SemanticRequest::new(root.clone(), targets_of(&output)));

    assert_eq!((outcome.diagnostics.as_slice(), outcome.fallbacks.as_slice()), (&[][..], &[][..]));
    let SemanticOutcome { components, .. } = outcome;
    let props: Vec<_> =
        components[0].props.iter().map(|prop| (prop.name.as_str(), &prop.prop_type, prop.parent.clone())).collect();
    let parent =
        |name: &str, file_name: &str| Some(PropParent { name: name.to_string(), file_name: file_name.to_string() });
    let own = parent("LibProps", root.join("Lib.tsx").as_str());
    assert_eq!(
        props,
        [
            ("when", &PropType::Named { name: "Date".into(), args: vec![] }, own.clone()),
            ("partial", &PropType::Object(vec![ObjectField::new("x", PropType::Number, false, String::new())]), own),
            ("type", &PropType::String, parent("Event", "bundled:///libs/lib.dom.d.ts")),
        ]
    );
}
