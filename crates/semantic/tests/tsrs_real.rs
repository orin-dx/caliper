#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! The backend against a real `tsrs` build. Set `TSRS_BIN` to its `tsrs` binary (`node_modules/tsrs/bin/tsrs`, or
//! `node_modules/@tsrs/<platform>/tsrs`) to run it; without the variable every test returns at once.

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::PropType;
use caliper_docgen_core::types::semantic::merge_semantic;
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

fn tsrs_bin() -> Option<Utf8PathBuf> {
    let bin = std::env::var("TSRS_BIN").ok().filter(|bin| !bin.is_empty());
    if bin.is_none() {
        eprintln!("skipped: TSRS_BIN is not set");
    }
    bin.map(Utf8PathBuf::from)
}

#[test]
fn tsrs_marks_an_unresolved_name_degraded_and_a_written_any_not() {
    let Some(bin) = tsrs_bin() else { return };
    let dir = tempfile::Builder::new().prefix("TsrsReal").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixture = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/errors");
    for name in ["Errors.tsx", "tsconfig.json"] {
        std::fs::copy(fixture.join(name), root.join(name)).unwrap();
    }
    let mut output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let mut request = SemanticRequest::new(root, targets_of(&output));
    request.tsc_path = Some(bin);

    let outcome = resolve(request);
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
