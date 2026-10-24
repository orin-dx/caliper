use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::pipeline::{extract, PipelineOptions};

/// A project with an installed package `ui-kit` whose types re-export through `export *`, a namespace re-export, a
/// star-export cycle, an enum and a component; a Radix-shaped `dialog-kit` exporting components under local aliases;
/// a JavaScript-only package; and `class-variance-authority`'s real `VariantProps` declaration.
fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    let base: &[(&str, &str)] = &[
        ("node_modules/ui-kit/package.json", r#"{ "name": "ui-kit", "version": "1.0.0", "types": "dist/index.d.ts" }"#),
        (
            "node_modules/ui-kit/dist/index.d.ts",
            "export * from './button';\nexport * from './cycle-a';\nexport * as Tokens from './tokens';\n\
             export { Tone } from './enum';\n",
        ),
        (
            "node_modules/ui-kit/dist/button.d.ts",
            "import * as React from 'react';\nimport type { Size } from './tokens';\n\
             export interface ButtonProps { size?: Size; label: string }\n\
             export declare const Button: React.ForwardRefExoticComponent<ButtonProps>;\n",
        ),
        ("node_modules/ui-kit/dist/tokens.d.ts", "export type Size = 'sm' | 'md' | 'lg';\nexport type Gap = number;\n"),
        ("node_modules/ui-kit/dist/cycle-a.d.ts", "export * from './cycle-b';\nexport type A = string;\n"),
        ("node_modules/ui-kit/dist/cycle-b.d.ts", "export * from './cycle-a';\nexport type B = boolean;\n"),
        ("node_modules/ui-kit/dist/enum.d.ts", "export declare enum Tone { Info = \"info\", Warn = \"warn\" }\n"),
        (
            "node_modules/dialog-kit/package.json",
            r#"{ "name": "dialog-kit", "version": "1.0.0", "types": "dist/index.d.ts" }"#,
        ),
        (
            "node_modules/dialog-kit/dist/index.d.ts",
            "import * as React from 'react';\n\
             interface DialogContentProps { forceMount?: true; onEscapeKeyDown?: (event: KeyboardEvent) => void }\n\
             declare const DialogContent: React.ForwardRefExoticComponent<DialogContentProps>;\n\
             export { DialogContent as Content, DialogContent, type DialogContentProps };\n",
        ),
        ("node_modules/plain-js/package.json", r#"{ "name": "plain-js", "version": "1.0.0", "main": "index.js" }"#),
        ("node_modules/plain-js/index.js", "module.exports = {};\n"),
        (
            "node_modules/class-variance-authority/package.json",
            r#"{ "name": "class-variance-authority", "version": "0.7.0", "types": "dist/index.d.ts" }"#,
        ),
        (
            "node_modules/class-variance-authority/dist/index.d.ts",
            "type OmitUndefined<T> = T extends undefined ? never : T;\n\
             export type VariantProps<Component extends (...args: any) => any> = \
             Omit<OmitUndefined<Parameters<Component>[0]>, \"class\" | \"className\">;\n\
             export declare const cva: (base?: string, config?: unknown) => (props?: unknown) => string;\n",
        ),
    ];
    for (path, body) in base.iter().chain(files) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    (tmp, root)
}

fn options(root: &Utf8Path, load: bool) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![root.join("src")],
        cache_dir: Some(root.join("cache")),
        load_dependencies: load,
        ..Default::default()
    }
}

fn prop_types(output: &crate::types::ExtractionOutput, component: &str) -> Vec<(String, String)> {
    output.components[component].props.values().map(|p| (p.name.clone(), p.prop_type.raw_string())).collect()
}

fn demand(root: &Utf8Path, specifier: &str, name: &str) -> Demand {
    Demand { from: root.join("src/Badge.tsx"), specifier: specifier.into(), name: name.into() }
}

const BADGE: &str = "import type { ButtonProps, B, Tokens, Tone } from 'ui-kit';\nimport { Button } from 'ui-kit';\n\
    import * as React from 'react';\n\
    export function Badge(props: ButtonProps & { b?: B; gap?: Tokens.Gap; tone?: Tone }) { return null; }\n\
    export function Wrapped(props: React.ComponentProps<typeof Button>) { return null; }\n";

#[test]
fn package_types_resolve_through_star_exports_cycles_and_namespaces_and_the_value_index() {
    let (_tmp, root) = project(&[("src/Badge.tsx", BADGE)]);

    let output = extract(&options(&root, true));

    assert_eq!(output.components.keys().collect::<Vec<_>>(), ["Badge", "Wrapped"], "no dependency component leaks");
    assert_eq!(output.enums, Default::default(), "no dependency enum leaks");
    assert_eq!(
        prop_types(&output, "Badge"),
        [
            ("b".into(), "boolean".into()),
            ("gap".into(), "number".into()),
            ("label".into(), "string".into()),
            ("size".into(), "\"sm\" | \"md\" | \"lg\"".into()),
            ("tone".into(), "Tone".into()),
        ]
    );
    assert_eq!(
        prop_types(&output, "Wrapped"),
        [("label".into(), "string".into()), ("size".into(), "\"sm\" | \"md\" | \"lg\"".into())]
    );
}

#[test]
fn component_props_of_a_namespace_member_resolve_through_a_local_export_alias() {
    let source = "import * as DialogPrimitive from 'dialog-kit';\nimport * as React from 'react';\n\
        export function DialogContent(props: React.ComponentProps<typeof DialogPrimitive.Content>) { return null; }\n";
    let (_tmp, root) = project(&[("src/Dialog.tsx", source)]);

    let output = extract(&options(&root, true));

    assert_eq!(output.components.keys().collect::<Vec<_>>(), ["DialogContent"]);
    assert_eq!(
        prop_types(&output, "DialogContent"),
        [("forceMount".into(), "true".into()), ("onEscapeKeyDown".into(), "(event: KeyboardEvent) => void".into())]
    );
}

#[test]
fn without_the_flag_package_types_stay_unresolved() {
    let (_tmp, root) = project(&[("src/Badge.tsx", BADGE)]);

    let output = extract(&options(&root, false));

    assert_eq!(
        prop_types(&output, "Badge"),
        [("b".into(), "B".into()), ("gap".into(), "Tokens.Gap".into()), ("tone".into(), "Tone".into())]
    );
}

/// cva's `VariantProps` is `Omit<OmitUndefined<Parameters<C>[0]>, …>`, which needs inference: loaded, it evaluates to
/// nothing, so the shortcut stands in for it (ADR 0014).
#[test]
fn a_shortcut_stands_in_for_a_loaded_declaration_that_evaluates_to_nothing() {
    let source = "import { cva, type VariantProps } from 'class-variance-authority';\n\
        const variants = cva('', { variants: { intent: { primary: '', danger: '' } } });\n\
        export function Pill(props: VariantProps<typeof variants>) { return null; }\n";
    let (_tmp, root) = project(&[("src/Pill.tsx", source)]);

    let off = extract(&options(&root, false));
    let on = extract(&options(&root, true));

    assert_eq!(prop_types(&on, "Pill"), prop_types(&off, "Pill"));
    assert_eq!(prop_types(&on, "Pill"), [("intent".into(), "\"primary\" | \"danger\" | null".into())]);
}

/// A loaded declaration that resolves wins over a shortcut of the same name (ADR 0014); without it, the shortcut (an
/// HTML layer for the element) is the fallback.
#[test]
fn a_loaded_declaration_wins_over_a_shortcut_of_the_same_name() {
    let (_tmp, root) = project(&[
        (
            "node_modules/style-kit/package.json",
            r#"{ "name": "style-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
        ),
        ("node_modules/style-kit/index.d.ts", "export interface HTMLChakraProps<T> { tone?: \"calm\" | \"loud\" }\n"),
        (
            "src/Panel.tsx",
            "import type { HTMLChakraProps } from 'style-kit';\n\
             export interface PanelProps extends HTMLChakraProps<'div'> { label: string }\n\
             export function Panel(props: PanelProps) { return null; }\n",
        ),
    ]);

    let on = extract(&options(&root, true));
    let off = extract(&options(&root, false));

    assert_eq!(
        prop_types(&on, "Panel"),
        [("label".into(), "string".into()), ("tone".into(), "\"calm\" | \"loud\"".into())]
    );
    assert_eq!(prop_types(&off, "Panel"), [("label".into(), "string".into())]);
    let layers: Vec<Option<&str>> =
        off.components["Panel"].inheritance.iter().map(|layer| layer.html_element.as_deref()).collect();
    assert_eq!(layers, [Some("div")]);
}

/// `z.infer<typeof schema>` indexes the type of a runtime value, which no static reading evaluates: it keeps its name
/// whether or not zod's declaration is loaded, as TypeScript prints it, while one over a declared type still expands.
#[test]
fn an_alias_over_an_unevaluable_indexed_access_keeps_its_name() {
    let source = "import * as z from 'zod';\nimport * as React from 'react';\n\
        const schema = z.object({ id: 1 });\n\
        interface Parsed { _output: { id: number } }\n\
        export function Row(props: { item: z.infer<typeof schema>; items: z.infer<typeof schema>[]; parsed: z.infer<Parsed> }) { return null; }\n";
    let (_tmp, root) = project(&[
        ("node_modules/zod/package.json", r#"{ "name": "zod", "version": "3.23.8", "types": "index.d.ts" }"#),
        (
            "node_modules/zod/index.d.ts",
            "export interface ZodType<Output = any> { readonly _output: Output }\n\
             export declare function object<T>(shape: T): ZodType<T>;\n\
             export type infer<T extends { _output: unknown }> = T[\"_output\"];\n",
        ),
        ("src/Row.tsx", source),
    ]);

    let off = extract(&options(&root, false));
    let on = extract(&options(&root, true));

    assert_eq!(
        prop_types(&on, "Row"),
        [
            ("item".into(), "z.infer<typeof schema>".into()),
            ("items".into(), "z.infer<typeof schema>[]".into()),
            ("parsed".into(), "{ id: number }".into()),
        ]
    );
    assert_eq!(prop_types(&off, "Row")[..2], prop_types(&on, "Row")[..2]);
}

#[test]
fn custom_conditions_from_the_governing_tsconfig_pick_the_export_target() {
    let files: &[(&str, &str)] = &[
        ("tsconfig.json", "{ \"compilerOptions\": { \"customConditions\": [\"source\"] } }\n"),
        (
            "node_modules/cond-pkg/package.json",
            r#"{ "name": "cond-pkg", "version": "1.0.0", "exports": { ".": { "source": "./src/index.ts", "types": "./dist/index.d.ts" } } }"#,
        ),
        ("node_modules/cond-pkg/src/index.ts", "export interface CondProps { fromSource: string }\n"),
        ("node_modules/cond-pkg/dist/index.d.ts", "export interface CondProps { fromDist: string }\n"),
        (
            "src/Cond.tsx",
            "import type { CondProps } from 'cond-pkg';\nexport function Cond(props: CondProps) { return null; }\n",
        ),
    ];
    let (_tmp, root) = project(files);

    let output = extract(&options(&root, true));

    assert_eq!(prop_types(&output, "Cond"), [("fromSource".into(), "string".into())]);
}

/// The layer keeps one answer per `(module, name)` for names found through a star-export cycle, loads only declaring
/// files, and records why each demand it could not satisfy failed.
#[test]
fn the_loader_flattens_star_exports_and_cycles_and_records_why_a_demand_failed() {
    let (_tmp, root) = project(&[("src/Badge.tsx", BADGE)]);
    let demands = vec![
        demand(&root, "ui-kit", "B"),
        demand(&root, "ui-kit", "ButtonProps"),
        demand(&root, "ui-kit", "Missing"),
        demand(&root, "not-installed", "X"),
        demand(&root, "plain-js", "Y"),
    ];

    let layer = load(demands, &options(&root, true), None);

    let dist = root.join("node_modules/ui-kit/dist");
    let index = dist.join("index.d.ts");
    assert_eq!(layer.export_of(&index, "B"), Some(&Some((dist.join("cycle-b.d.ts"), "B".into()))));
    assert_eq!(layer.export_of(&dist.join("cycle-a.d.ts"), "B"), Some(&Some((dist.join("cycle-b.d.ts"), "B".into()))));
    assert_eq!(layer.export_of(&index, "ButtonProps"), Some(&Some((dist.join("button.d.ts"), "ButtonProps".into()))));
    // `Missing` was looked for around the cycle, so its answer depended on the walk and was not kept.
    assert_eq!(layer.export_of(&index, "Missing"), None);
    let mut files: Vec<&str> = layer.files.iter().map(std::borrow::Borrow::<str>::borrow).collect();
    files.sort_unstable();
    assert_eq!(
        files,
        [dist.join("button.d.ts").as_str(), dist.join("cycle-b.d.ts").as_str(), dist.join("tokens.d.ts").as_str()]
    );
    let src = root.join("src");
    assert_eq!(layer.unresolved_reason(&src, "ui-kit", "Missing"), Some(Unresolved::NotExported));
    assert_eq!(layer.unresolved_reason(&src, "not-installed", "X"), Some(Unresolved::Unresolvable));
    assert_eq!(layer.unresolved_reason(&src, "plain-js", "Y"), Some(Unresolved::NoTypes));
    assert_eq!(layer.unresolved_reason(&src, "ui-kit", "B"), None);
    // A declaring file enters whole, so its components join the value index even when only a type was demanded.
    assert_eq!(
        layer.component(&dist.join("button.d.ts"), "Button").map(|m| m.props_type_name.as_str()),
        Some("ButtonProps")
    );
}

/// The same demands give the same layer in any order and on any thread count.
#[test]
fn the_loader_is_deterministic_across_demand_order_and_threads() {
    let (_tmp, root) = project(&[("src/Badge.tsx", BADGE)]);
    let names = ["B", "A", "ButtonProps", "Tokens.Gap", "Tone", "Button", "Missing"];
    let snapshot = |reverse: bool, threads: usize| {
        let mut demands: Vec<Demand> = names.iter().map(|name| demand(&root, "ui-kit", name)).collect();
        if reverse {
            demands.reverse();
        }
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        // No index: each run walks the stubs itself.
        let options = PipelineOptions {
            cache_dir: Some(root.join(format!("cache-{reverse}-{threads}"))),
            ..options(&root, true)
        };
        let layer = pool.install(|| load(demands, &options, None));
        let mut exports: Vec<_> =
            layer.exports.iter().map(|((m, n), found)| (m.clone(), n.clone(), found.clone())).collect();
        exports.sort();
        let mut files: Vec<PathKey> = layer.files.iter().cloned().collect();
        files.sort();
        let mut unresolved: Vec<_> = layer.unresolved.iter().map(|(k, v)| (k.clone(), *v)).collect();
        unresolved.sort();
        (
            exports,
            files,
            unresolved,
            layer.global.component_mappings.iter().map(|m| m.component_name.clone()).collect::<Vec<_>>(),
        )
    };

    let first = snapshot(false, 1);
    for (reverse, threads) in [(true, 1), (false, 4), (true, 18)] {
        assert_eq!(snapshot(reverse, threads), first, "reverse {reverse}, {threads} threads");
    }
}

#[test]
fn output_is_the_same_on_any_thread_count_and_with_a_cold_or_warm_index() {
    let (_tmp, root) = project(&[("src/Badge.tsx", BADGE)]);
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        let output = pool.install(|| extract(&options(&root, true)));
        (output.components, output.enums, output.diagnostics)
    };

    let cold = run(1);
    let indexes: Vec<String> = root
        .join("cache")
        .read_dir_utf8()
        .unwrap()
        .map(|e| e.unwrap().file_name().to_owned())
        .filter(|name| name.starts_with("dependency-index-"))
        .collect();
    assert_eq!(indexes.len(), 1, "one index file: {indexes:?}");
    for threads in [4, 1, 18] {
        assert_eq!(run(threads), cold, "{threads} threads, warm index");
    }
}

/// A package's changed `package.json` invalidates its answers, so an updated install is walked again.
#[test]
fn a_changed_package_is_walked_again_instead_of_answered_from_the_index() {
    let source =
        "import type { ButtonProps } from 'ui-kit';\nexport function Badge(props: ButtonProps) { return null; }\n";
    let (_tmp, root) = project(&[("src/Badge.tsx", source)]);
    let first = extract(&options(&root, true));
    assert_eq!(
        prop_types(&first, "Badge"),
        [("label".into(), "string".into()), ("size".into(), "\"sm\" | \"md\" | \"lg\"".into())]
    );

    let dist = root.join("node_modules/ui-kit/dist");
    std::fs::write(dist.join("index.d.ts"), "export * from './v2';\n").unwrap();
    std::fs::write(dist.join("v2.d.ts"), "export interface ButtonProps { v2: number }\n").unwrap();
    std::fs::write(
        root.join("node_modules/ui-kit/package.json"),
        r#"{ "name": "ui-kit", "version": "2.0.0", "types": "dist/index.d.ts" }"#,
    )
    .unwrap();

    let second = extract(&options(&root, true));
    assert_eq!(prop_types(&second, "Badge"), [("v2".into(), "number".into())]);
}

#[test]
fn an_unresolved_package_type_says_why_in_its_diagnostic() {
    let source = "import type { Missing } from 'ui-kit';\nimport type { Gone } from 'not-installed';\n\
        export function Badge(props: { a: Missing; b: Gone }) { return null; }\n";
    let (_tmp, root) = project(&[("src/Badge.tsx", source)]);

    let output = extract(&options(&root, true));

    let mut help: Vec<&str> = output
        .diagnostics
        .iter()
        .filter(|d| d.code == crate::DiagnosticCode::UnresolvableImport)
        .filter_map(|d| d.help.as_deref())
        .filter(|help| help.starts_with("Dependency types"))
        .collect();
    help.sort_unstable();
    assert_eq!(
        help,
        [
            "Dependency types are loaded, but for 'Gone' its module could not be resolved.",
            "Dependency types are loaded, but for 'Missing' its package's types do not export it.",
        ]
    );
}

/// Each edit's watch snapshot equals a cold extract of the same files: a new package name loads into a new layer, a
/// removed one leaves loaded declarations that change nothing, and a local declaration wins over the package's.
#[test]
fn watch_revisions_with_dependencies_match_a_cold_extract() {
    let first =
        "import type { ButtonProps } from 'ui-kit';\nexport function Badge(props: ButtonProps) { return null; }\n";
    let (_tmp, root) = project(&[("src/Badge.tsx", first)]);
    let badge = root.join("src/Badge.tsx");
    let session = crate::pipeline::WatchSession::new(options(&root, true));
    let _initial = session.initialize();
    let edits = [
        "import type { ButtonProps, A } from 'ui-kit';\nexport function Badge(props: ButtonProps & { a: A }) { return null; }\n",
        "export function Badge(props: { plain: string }) { return null; }\n",
        "interface ButtonProps { local: number }\nexport function Badge(props: ButtonProps) { return null; }\n",
        first,
    ];
    let expected: [&[&str]; 4] = [&["a", "label", "size"], &["plain"], &["local"], &["label", "size"]];
    for (edit, expected) in edits.into_iter().zip(expected) {
        std::fs::write(&badge, edit).unwrap();
        let _update = session.update_file(&badge);
        let watched = session.snapshot();
        let cold = extract(&options(&root, true));
        assert_eq!(watched.components, cold.components, "after:\n{edit}");
        assert_eq!(watched.enums, cold.enums, "after:\n{edit}");
        assert_eq!(watched.diagnostics, cold.diagnostics, "after:\n{edit}");
        assert_eq!(watched.components["Badge"].props.keys().collect::<Vec<_>>(), expected, "after:\n{edit}");
    }
}

// ─── collapseInheritedProps (ADR 0012) ────────────────────────────────────────

/// `big` declares `BigProps` (31 props, two of them notable), `SmallProps` (1) and `BigAlias` (31); the project declares
/// `LocalBig` (31), and components inheriting each.
fn collapse_project() -> (tempfile::TempDir, Utf8PathBuf) {
    let many = |prefix: &str| (0..29).map(|i| format!("{prefix}{i}?: string; ")).collect::<String>();
    let big = format!(
        "export interface BigProps {{ {}onClick?: () => void; className?: string }}\n\
         export interface SmallProps {{ only: string }}\nexport type BigAlias = {{ {}id?: string; style?: object }};\n",
        many("p"),
        many("q")
    );
    let local = format!("export interface LocalBig {{ {}l29?: string; l30?: string }}\n", many("l"));
    let files = [
        ("node_modules/big/package.json", r#"{ "name": "big", "version": "1.0.0", "types": "index.d.ts" }"#.to_owned()),
        ("node_modules/big/index.d.ts", big),
        ("src/local.ts", local),
        (
            "src/Big.tsx",
            "import type { BigProps, SmallProps, BigAlias } from 'big';\nimport type { LocalBig } from './local';\n\
             export interface FieldProps extends BigProps, SmallProps { own: number }\n\
             export function Field(props: FieldProps) { return null; }\n\
             export function Aliased(props: BigAlias & { own: number }) { return null; }\n\
             export function Direct(props: BigProps) { return null; }\n\
             export function Picked(props: Pick<BigProps, 'p0' | 'p1'>) { return null; }\n\
             export function Keyed(props: Omit<LocalBig, keyof BigProps | 'l30'>) { return null; }\n\
             export interface LocalField extends LocalBig { own: number }\n\
             export function Local(props: LocalField) { return null; }\n\
             export function OwnBig(props: LocalBig) { return null; }\n"
                .to_owned(),
        ),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(p, b)| (*p, b.as_str())).collect();
    project(&files)
}

fn with_collapse(root: &Utf8Path, mode: crate::pipeline::CollapseInheritedProps) -> crate::types::ExtractionOutput {
    extract(&PipelineOptions { collapse_inherited_props: mode, ..options(root, true) })
}

fn keys<V>(map: &std::collections::BTreeMap<String, V>) -> Vec<&str> {
    map.keys().map(String::as_str).collect()
}

fn layers(entry: &crate::types::ComponentEntry) -> Vec<(&str, u32, Option<&str>)> {
    entry.inheritance.iter().map(|l| (l.type_name.as_str(), l.total_props, l.html_element.as_deref())).collect()
}

#[test]
fn a_large_dependency_source_collapses_to_a_layer_and_its_notable_props() {
    use crate::pipeline::CollapseInheritedProps;
    let (_tmp, root) = collapse_project();

    let output = with_collapse(&root, CollapseInheritedProps::Dependencies);

    let field = &output.components["Field"];
    assert_eq!(keys(&field.props), ["only", "own"]);
    assert_eq!(layers(field), [("BigProps", 31, None)]);
    assert_eq!(field.inheritance[0].file_name, root.join("node_modules/big/index.d.ts").as_str());
    assert_eq!(keys(&field.notable_inherited), ["className", "onClick"]);
    assert_eq!(field.notable_inherited["onClick"].prop_type.raw_string(), "() => void");

    let aliased = &output.components["Aliased"];
    assert_eq!(keys(&aliased.props), ["own"]);
    assert_eq!(layers(aliased), [("BigAlias", 31, None)]);
    assert_eq!(keys(&aliased.notable_inherited), ["id", "style"]);

    // The component's own props type, a `Pick` from a large source and a `keyof` operand are never collapsed.
    assert_eq!(output.components["Direct"].props.len(), 31);
    assert_eq!(output.components["Direct"].inheritance, []);
    assert_eq!(keys(&output.components["Picked"].props), ["p0", "p1"]);
    let mut kept: Vec<String> = (0..30).map(|i| format!("l{i}")).collect();
    kept.sort();
    assert_eq!(keys(&output.components["Keyed"].props), kept);
    // A project source stays expanded by default.
    assert_eq!(output.components["Local"].props.len(), 32);
    assert_eq!(output.components["Local"].inheritance, []);
}

#[test]
fn all_collapses_inherited_project_sources_too_and_none_expands_everything() {
    use crate::pipeline::CollapseInheritedProps;
    let (_tmp, root) = collapse_project();

    let all = with_collapse(&root, CollapseInheritedProps::All);
    let none = with_collapse(&root, CollapseInheritedProps::None);
    let default = with_collapse(&root, CollapseInheritedProps::Dependencies);

    assert_eq!(keys(&all.components["Local"].props), ["own"]);
    assert_eq!(layers(&all.components["Local"]), [("LocalBig", 31, None)]);
    assert_eq!(keys(&all.components["Local"].notable_inherited), Vec::<&str>::new());
    assert_eq!(all.components["OwnBig"].props.len(), 31, "the component's own props type");
    assert_eq!(keys(&all.components["Field"].props), ["only", "own"]);

    for name in ["Field", "Aliased", "Local", "Direct"] {
        assert_eq!(none.components[name].inheritance, [], "{name}");
        assert_eq!(none.components[name].notable_inherited.len(), 0, "{name}");
    }
    assert_eq!(none.components["Field"].props.len(), 33);
    assert_eq!(none.components["Aliased"].props.len(), 32);
    // Collapsing only hides props; whatever stays expanded matches a full expansion exactly.
    for name in ["Direct", "Picked", "Keyed", "Local", "OwnBig"] {
        assert_eq!(default.components[name], none.components[name], "{name}");
    }
    for (name, prop) in &default.components["Field"].props {
        assert_eq!(prop, &none.components["Field"].props[name], "{name}");
    }
}

/// Without loaded dependencies nothing is a dependency source, so the default collapses nothing.
#[test]
fn the_default_changes_nothing_without_loaded_dependencies() {
    use crate::pipeline::CollapseInheritedProps;
    let (_tmp, root) = collapse_project();
    let off = |mode| {
        let output = extract(&PipelineOptions { collapse_inherited_props: mode, ..options(&root, false) });
        (output.components, output.enums, output.diagnostics)
    };

    assert_eq!(off(CollapseInheritedProps::Dependencies), off(CollapseInheritedProps::None));
}
