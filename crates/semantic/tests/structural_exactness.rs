#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The structural lane never expands `Partial`, `Required`, `Pick`, `Omit` or `NonNullable`: each application stays a
//! named type, and the linked checker is the one that answers it with the object TypeScript computes.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::PropType;
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

const SOURCE: &str = r#"
interface Plain { a: string; b?: number }
export function Box(props: {
  partial?: Partial<Plain>;
  required?: Required<Plain>;
  pick?: Pick<Plain, "a">;
  omit?: Omit<Plain, "a">;
  nonNullable?: NonNullable<Plain | null>;
}) { return null; }
"#;

const PROPS: [&str; 5] = ["partial", "required", "pick", "omit", "nonNullable"];

#[test]
fn structural_never_expands_a_utility_and_the_checker_does() {
    let dir = tempfile::Builder::new().prefix("Exactness").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    std::fs::write(root.join("lib.tsx"), SOURCE).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true, "jsx": "react-jsx", "module": "esnext", "moduleResolution": "bundler" } }"#,
    )
    .unwrap();

    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let entry = &output.components["Box"];
    for name in PROPS {
        assert!(
            matches!(entry.props[name].prop_type, PropType::Named { .. }),
            "{name} stays a named application: {}",
            entry.props[name].prop_type.raw_string()
        );
    }

    let mut request = SemanticRequest::new(root, targets_of(&output));
    request.request_timeout = Duration::from_secs(60);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(resolve(request)).ok());
    let outcome = receiver.recv_timeout(Duration::from_secs(120)).expect("the conversion ends");
    assert_eq!(outcome.diagnostics, []);
    let answer = outcome.components.iter().find(|component| component.key == "Box").expect("the checker answers Box");
    for name in ["partial", "required", "pick", "omit"] {
        let prop = answer.props.iter().find(|candidate| candidate.name == name).expect("the checker gives the prop");
        assert!(
            matches!(prop.prop_type, PropType::Object(_)),
            "the checker expands {name}: {}",
            prop.prop_type.raw_string()
        );
    }
}
