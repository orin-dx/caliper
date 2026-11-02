#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A component re-bound from an import: `const { Item } = Imported` and `const Alert = InternalAlert`. An alias of a
//! package's component documents that component's props; an alias of a component the project declares under the same
//! name is that component, not a second one.

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions};
use caliper_docgen_core::types::{DetectionRule, ExtractionOutput};
use camino::Utf8PathBuf;
use tempfile::TempDir;

const PACKAGE_INDEX: &str = "import Menu from './Menu';\nexport { Memoed } from './Memoed';\nexport default Menu;\n";

const PACKAGE_MENU: &str = r#"
import * as React from 'react';
import type { ItemProps } from './Item';
import Item from './Item';
export interface MenuProps { open?: boolean }
declare const Menu: React.ForwardRefExoticComponent<MenuProps & React.RefAttributes<HTMLDivElement>> & {
    Item: typeof Item;
};
export default Menu;
"#;

const PACKAGE_ITEM: &str = r#"
import * as React from 'react';
export interface ItemProps {
    /** The value the item stands for. */
    value: string;
}
declare const Item: React.FC<ItemProps>;
export default Item;
"#;

const PACKAGE_MEMOED: &str = r#"
import * as React from 'react';
export interface MemoedProps {
    /** Pixels. */
    size?: number;
}
export declare const Memoed: React.MemoExoticComponent<React.FC<MemoedProps>>;
"#;

const ALERT: &str = r#"
import * as React from 'react';
export interface AlertProps { message: string }
const Alert = (props: AlertProps) => <div>{props.message}</div>;
export default Alert;
"#;

const ALERT_INDEX: &str = r#"
import InternalAlert from './Alert';
export const Alert = InternalAlert;
"#;

const MENU_ITEMS: &str = r#"
import Menu from 'menupkg';
export const { Item } = Menu;
export const { Item: Row } = Menu;
import { Memoed } from 'menupkg';
export const Fast = Memoed;
"#;

struct Project {
    _tmp: TempDir,
    root: Utf8PathBuf,
}

fn project() -> Project {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    for (name, source) in [
        ("src/Alert.tsx", ALERT),
        ("src/alert-index.tsx", ALERT_INDEX),
        ("src/menu-items.tsx", MENU_ITEMS),
        ("node_modules/menupkg/package.json", r#"{"name":"menupkg","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/menupkg/index.d.ts", PACKAGE_INDEX),
        ("node_modules/menupkg/Menu.d.ts", PACKAGE_MENU),
        ("node_modules/menupkg/Item.d.ts", PACKAGE_ITEM),
        ("node_modules/menupkg/Memoed.d.ts", PACKAGE_MEMOED),
    ] {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    Project { _tmp: tmp, root }
}

fn run(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions {
        src_dirs: vec![project.root.join("src")],
        cache_dir: Some(project.root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        load_dependencies: true,
        ..Default::default()
    })
}

#[test]
fn a_member_destructured_from_a_packages_component_has_that_members_props() {
    let project = project();
    let output = run(&project);
    for name in ["Item", "Row"] {
        let entry =
            output.components.get(name).unwrap_or_else(|| panic!("{name} missing: {:?}", output.components.keys()));
        assert_eq!(entry.detected_by, DetectionRule::ImportedMemberAlias, "{name}");
        assert_eq!(entry.props.keys().collect::<Vec<_>>(), ["value"], "{name}");
        assert_eq!(entry.props["value"].description, "The value the item stands for.", "{name}");
    }
}

#[test]
fn an_alias_of_a_component_the_project_declares_under_that_name_is_not_a_second_component() {
    let project = project();
    let output = run(&project);
    let alerts: Vec<&String> = output.components.keys().filter(|key| key.starts_with("Alert")).collect();
    assert_eq!(alerts.len(), 1, "got {alerts:?}");
    let alert = &output.components["Alert"];
    assert_ne!(alert.detected_by, DetectionRule::ImportedMemberAlias);
    assert_eq!(alert.props.keys().collect::<Vec<_>>(), ["message"]);
}

#[test]
fn an_alias_of_a_memoized_component_has_the_props_of_the_component_it_wraps() {
    let project = project();
    let output = run(&project);
    let fast = output.components.get("Fast").unwrap_or_else(|| panic!("missing: {:?}", output.components.keys()));
    assert_eq!(fast.props.keys().collect::<Vec<_>>(), ["size"]);
    assert_eq!(fast.props["size"].description, "Pixels.");
}
