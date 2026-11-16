use camino::Utf8PathBuf;
use tempfile::TempDir;

use crate::pipeline::extract;
use crate::types::options::{CollapseInheritedProps, PipelineOptions};
use crate::types::{ExtractionOutput, PropType};

/// Extract one `.tsx` file in a temp project with an empty `typescript` install.
fn extract_file(source: &str) -> ExtractionOutput {
    extract_file_collapsing(source, CollapseInheritedProps::default())
}

fn extract_file_collapsing(source: &str, collapse_inherited_props: CollapseInheritedProps) -> ExtractionOutput {
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
        collapse_inherited_props,
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
fn a_render_callbacks_discriminated_union_keeps_its_discriminant() {
    let output = extract_file(&format!(
        "{LEAF_FACTORY}
interface A {{ mode: 'a'; left: string }}
interface B {{ mode: 'b'; right: number }}
type Props = A | B;
export const Pair = createLeaf(SeparatorNode, (props: Props, ref) => <div />);
"
    ));
    assert_eq!(output.components["Pair"].discriminant_prop.as_deref(), Some("mode"));
}

#[test]
fn a_render_callback_type_parameter_does_not_shadow_a_declaration_in_the_result() {
    let output = extract_file(
        r#"
interface Base { label: string }
declare function make<P>(tag: P, render: unknown): React.FC<Base>;
export const Widget = make('div', <Base,>(props: { value: Base }) => <div />);
"#,
    );
    assert!(output.components.contains_key("Widget"), "{:?}", output.components.keys().collect::<Vec<_>>());
    assert!(prop_names(&output, "Widget").contains(&"label".to_owned()));
}

#[test]
fn a_render_callback_type_parameter_stays_a_placeholder_in_its_own_props() {
    let output = extract_file(
        r#"
declare function make<P>(tag: P, render: unknown): React.FC<{ id?: string }>;
export const Widget = make('div', <T,>(props: { v: T }) => <div />);
"#,
    );
    assert_eq!(prop_names(&output, "Widget"), ["id", "v"]);
    assert!(format!("{:?}", output.components["Widget"].props["v"].prop_type).contains('T'));
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

/// A local factory whose result has the props `{ id?: string }`, taking a callback as its last argument.
const LOCAL_FACTORY: &str = r#"
import type { ForwardRefExoticComponent } from "react";
declare function make<P>(tag: string, callback: (props: P, ref?: unknown) => unknown): ForwardRefExoticComponent<{ id?: string }>;
type Theme = { color: string; gap: number };
"#;

#[test]
fn an_annotated_callback_that_does_not_render_gives_no_props() {
    let output = extract_file(&format!(
        "{LOCAL_FACTORY}
export const AsString = make('div', (theme: Theme) => `${{theme.color}}`);
export const AsNumber = make('div', (theme: Theme) => theme.gap);
export const AsObject = make('div', (theme: Theme) => ({{ color: theme.color }}));
export const AsFunction = make('div', (theme: Theme) => () => theme.gap);
export const AsVoid = make('div', function (theme: Theme) {{ console.log(theme); }});
"
    ));
    for name in ["AsString", "AsNumber", "AsObject", "AsFunction", "AsVoid"] {
        assert!(output.components.contains_key(name), "{name} is a component");
        assert!(!output.components[name].props.contains_key("color"), "{name} took the callback's parameter as props");
    }
}

#[test]
fn an_async_generator_or_rest_callback_gives_no_props() {
    let output = extract_file(&format!(
        "{LOCAL_FACTORY}
export const Async = make('div', async (theme: Theme) => <div>{{theme.color}}</div>);
export const Generator = make('div', function* (theme: Theme) {{ yield <div />; }});
export const Rest = make('div', (...theme: Theme[]) => <div />);
"
    ));
    for name in ["Async", "Generator", "Rest"] {
        assert!(output.components.contains_key(name), "{name} is a component");
        assert!(!output.components[name].props.contains_key("color"), "{name} took the callback's parameter as props");
    }
}

#[test]
fn an_unannotated_render_callback_with_two_parameters_still_works() {
    let output = extract_file(&format!(
        "{LOCAL_FACTORY}
interface BoxProps {{ tone?: string }}
export const Box = make('div', (props: BoxProps, ref: unknown) => <div />);
"
    ));
    assert_eq!(prop_names(&output, "Box"), ["id", "tone"]);
}

#[test]
fn a_render_callback_annotated_with_a_large_named_interface_keeps_its_props() {
    let members: String = (0..210).map(|i| format!("  p{i}?: string;\n")).collect();
    let output = extract_file(&format!(
        "{LOCAL_FACTORY}
interface WideProps {{
{members}}}
export const Wide = make('div', (props: WideProps) => <div />);
"
    ));
    let names = prop_names(&output, "Wide");
    assert!(names.contains(&"p0".to_owned()) && names.contains(&"p209".to_owned()), "{}", names.len());
}

fn members(prefix: &str, count: usize) -> String {
    (0..count).map(|i| format!("  {prefix}{i}: string;\n")).collect()
}

#[test]
fn a_callbacks_named_props_type_lists_its_own_members_in_every_collapse_mode() {
    let own = members("m", 201);
    let named =
        format!("{LOCAL_FACTORY}\ninterface Big {{\n{own}}}\nexport const C = make('div', (props: Big) => <div />);\n");
    let inline = format!("{LOCAL_FACTORY}\nexport const C = make('div', (props: {{\n{own}}}) => <div />);\n");
    for mode in [CollapseInheritedProps::Dependencies, CollapseInheritedProps::All] {
        for source in [&named, &inline] {
            let output = extract_file_collapsing(source, mode);
            let names = prop_names(&output, "C");
            assert_eq!(names.iter().filter(|n| n.starts_with('m')).count(), 201, "{mode:?}");
        }
    }
}

#[test]
fn a_callbacks_named_props_type_lets_its_extended_source_collapse() {
    let base = members("b", 201);
    let own = members("m", 3);
    let output = extract_file_collapsing(
        &format!(
            "{LOCAL_FACTORY}\ninterface Base201 {{\n{base}}}\ninterface Big extends Base201 {{\n{own}}}\nexport const C = make('div', (props: Big) => <div />);\n"
        ),
        CollapseInheritedProps::Dependencies,
    );
    let names = prop_names(&output, "C");
    assert_eq!(names.iter().filter(|n| n.starts_with('m')).count(), 3, "{names:?}");
    assert_eq!(names.iter().filter(|n| n.starts_with('b')).count(), 0);
    assert!(output.components["C"].inheritance.iter().any(|layer| layer.type_name == "Base201"));
}

fn default_of(output: &ExtractionOutput, component: &str, prop: &str) -> Option<String> {
    output.components[component].props[prop].default_value.as_ref().map(|d| d.value.clone())
}

#[test]
fn a_render_callbacks_destructuring_default_is_the_props_default() {
    let output = extract_file(&format!(
        "{LOCAL_FACTORY}
export const Sized = make('div', ({{ size = 'M' }}: {{ size?: 'S' | 'M' }}) => <div />);
export const Plain = make('div', ({{ size }}: {{ size?: 'S' | 'M' }}) => <div />);
"
    ));
    assert_eq!(prop_names(&output, "Sized"), ["id", "size"]);
    assert_eq!(default_of(&output, "Sized", "size").as_deref(), Some("'M'"));
    assert_eq!(default_of(&output, "Plain", "size"), None);
}

#[test]
fn an_unannotated_callbacks_defaults_are_not_read() {
    let output = extract_file(&format!(
        "{LOCAL_FACTORY}
export const Loose = make('div', ({{ size = 'M' }}) => <div />);
export const NoRender = make('div', ({{ size = 'M' }}: {{ size?: string }}) => 'x');
"
    ));
    for name in ["Loose", "NoRender"] {
        assert!(!output.components[name].props.contains_key("size"), "{name} took the callback's props");
    }
}

#[test]
fn unreadable_destructuring_defaults_are_skipped() {
    let output = extract_file(&format!(
        "{LOCAL_FACTORY}
const key = 'size';
export const Odd = make('div', ({{ [key]: a = 1, tone: {{ x = 2 }} = {{}}, size: s = 'M', ...rest }}: {{ size?: string; tone?: {{ x?: number }} }}) => <div />);
"
    ));
    assert_eq!(default_of(&output, "Odd", "size").as_deref(), Some("'M'"));
    // The nested pattern's own default (`x = 2`) is no prop's default; the prop's is the `{}` written after it.
    assert_eq!(default_of(&output, "Odd", "tone").as_deref(), Some("{}"));
    assert!(!output.components["Odd"].props.contains_key("x"));
}
