use camino::Utf8PathBuf;
use tempfile::TempDir;

use crate::pipeline::extract;
use crate::types::options::PipelineOptions;
use crate::types::{ExtractionOutput, PropType};

/// Extract one `.tsx` file in a temp project with an empty `typescript` install.
fn extract_file(source: &str) -> ExtractionOutput {
    let tmp = TempDir::new().unwrap();
    let lib = tmp.path().join("node_modules/typescript/lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(
        tmp.path().join("node_modules/typescript/package.json"),
        r#"{"name":"typescript","version":"5.9.3"}"#,
    )
    .unwrap();
    for file in ["lib.es5.d.ts", "lib.dom.d.ts"] {
        std::fs::write(lib.join(file), "").unwrap();
    }
    std::fs::write(tmp.path().join("Component.tsx"), source).unwrap();
    extract(&PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    })
}

fn prop_names(output: &ExtractionOutput, component: &str) -> Vec<String> {
    output.components[component].props.keys().cloned().collect()
}

const TAGGED_FACTORY: &str = r#"
declare function styled(tag: string, config: unknown): unknown;
interface HTMLStyledProps<T extends string> { as?: T }
export interface CenterProps extends HTMLStyledProps<"div"> {}
export const Center = styled("div", {
  base: { display: "flex" },
  variants: {
    inline: { true: { display: "inline-flex" } },
    tone: toneVariants,
    placement: { start: { left: 0 }, end: { right: 0 } },
  },
});
"#;

#[test]
fn a_chakra_factory_variant_key_is_an_own_prop() {
    let output = extract_file(TAGGED_FACTORY);
    assert_eq!(prop_names(&output, "Center"), ["as", "inline", "placement"]);
    let props = &output.components["Center"].props;
    assert_eq!(props["inline"].prop_type, PropType::Boolean);
    assert!(!props["inline"].required);
    assert_eq!(props["placement"].prop_type.raw_string(), "\"start\" | \"end\"");
}

#[test]
fn a_factory_does_not_invent_ref() {
    let output = extract_file(TAGGED_FACTORY);
    assert!(!output.components["Center"].props.contains_key("ref"));
}

#[test]
fn a_variants_object_that_is_not_a_style_recipe_gives_no_props() {
    let output = extract_file(
        r#"
declare function styled(tag: string, config: unknown): unknown;
interface HTMLStyledProps<T extends string> { as?: T }
export interface FadeProps extends HTMLStyledProps<"div"> {}
export const Fade = styled("div", { variants: { visible: { opacity: 1 }, hidden: { opacity: 0 } } });
"#,
    );
    assert_eq!(prop_names(&output, "Fade"), ["as"]);
}

/// A callee declared elsewhere whose result is a component with no props of its own, as `createLeafComponent` reads.
const LEAF_FACTORY: &str = r#"
import type { ForwardRefExoticComponent, ForwardedRef } from "react";
declare const SeparatorNode: unknown;
function createLeaf<P>(node: unknown, render: (props: P, ref: ForwardedRef<HTMLElement>) => unknown): ForwardRefExoticComponent<{ id?: string }> {
  return null as any;
}
"#;

#[test]
fn a_factory_render_function_annotation_is_the_props_type() {
    let output = extract_file(&format!(
        "{LEAF_FACTORY}
export const Divider = createLeaf(
  SeparatorNode,
  function Divider({{size}}: {{size?: 'S' | 'M'}}, ref: ForwardedRef<HTMLElement>) {{
    return <hr ref={{ref}} />;
  }}
);
interface TriggerProps {{ children: unknown; delay?: number }}
export const Trigger = createLeaf(SeparatorNode, (props: TriggerProps, ref) => <div>{{props.delay}}</div>);
"
    ));
    assert_eq!(prop_names(&output, "Divider"), ["id", "size"]);
    assert_eq!(prop_names(&output, "Trigger"), ["children", "delay", "id"]);
    assert!(output.components["Trigger"].props["children"].required);
}

#[test]
fn a_hook_call_given_a_function_is_no_component() {
    let output = extract_file(
        r#"
declare function useMemo<T>(make: (deps: number) => T, deps: unknown[]): T;
export const Value = useMemo((n: number) => n + 1, []);
"#,
    );
    assert!(!output.components.contains_key("Value"), "{:?}", output.components.keys().collect::<Vec<_>>());
}

#[test]
fn a_call_that_is_not_a_component_stays_none_with_a_recipe_argument() {
    let output = extract_file(
        r#"
export function defineConfig<T>(config: T): T { return config; }
export const Theme = defineConfig({ variants: { size: { sm: { fontSize: 1 } } } });
"#,
    );
    assert!(!output.components.contains_key("Theme"), "{:?}", output.components.keys().collect::<Vec<_>>());
}

#[test]
fn a_forward_ref_component_is_unchanged() {
    let output = extract_file(
        r#"
import { forwardRef } from "react";
interface ButtonProps { label?: string }
export const Button = forwardRef(function Button(props: ButtonProps, ref) { return <button ref={ref} />; });
"#,
    );
    assert_eq!(prop_names(&output, "Button"), ["label", "ref"]);
}
