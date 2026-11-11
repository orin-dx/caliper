#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! An anonymous default export takes its name from its file. A named export of that name is another component, so each
//! has its own key and its own exports whatever order the module writes them in, and a default export a call wraps is
//! an export of the component the call wraps.

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, extract};
use camino::Utf8PathBuf;
use tempfile::TempDir;

const PRELUDE: &str = r#"
import { forwardRef } from 'react';

interface NamedProps { named: string }
interface DefaultProps { fallback: string }
"#;

const NAMED: &str = "export const E = forwardRef<HTMLDivElement, NamedProps>((props, ref) => <div ref={ref} />);\n";
const DEFAULT: &str = "export default forwardRef<HTMLDivElement, DefaultProps>((props, ref) => <i ref={ref} />);\n";

/// Each component's key, its props and its exports.
fn components(file: &str, source: &str) -> BTreeMap<String, (Vec<String>, Vec<String>)> {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join(file), source).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        ..Default::default()
    };
    extract(&options)
        .components
        .into_iter()
        .map(|(key, entry)| {
            let exports = entry.exports.iter().map(ToString::to_string).collect();
            (key, (entry.props.keys().cloned().collect(), exports))
        })
        .collect()
}

fn row(props: &str, exports: &str) -> (Vec<String>, Vec<String>) {
    (vec![props.to_owned()], vec![exports.to_owned()])
}

#[test]
fn a_default_export_beside_a_named_export_of_its_file_name_is_a_component_of_its_own() {
    let expected =
        BTreeMap::from([("E".to_owned(), row("named", "E")), ("E (default)".to_owned(), row("fallback", "default"))]);
    for body in [format!("{NAMED}{DEFAULT}"), format!("{DEFAULT}{NAMED}")] {
        assert_eq!(components("E.tsx", &format!("{PRELUDE}{body}")), expected);
    }
}

#[test]
fn a_default_export_keeps_the_name_of_its_file_when_no_component_has_it() {
    let source = format!("{PRELUDE}{DEFAULT}");
    assert_eq!(components("E.tsx", &source), BTreeMap::from([("E".to_owned(), row("fallback", "default"))]));
}

#[test]
fn a_default_export_a_call_wraps_is_the_export_of_the_component_it_wraps() {
    let source = r#"
import * as React from 'react';

interface TableProps { rows: string[] }

const InternalTable = (props: TableProps) => <table />;

export default React.forwardRef(InternalTable) as RefTable;
"#;
    assert_eq!(components("Table.tsx", source), BTreeMap::from([("InternalTable".to_owned(), row("rows", "default"))]));
}
