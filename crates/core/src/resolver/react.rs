//! React builtin type mapping and related helpers.

use camino::Utf8Path;
use oxc_resolver::Resolver;

use crate::types::*;

use super::collected::resolve_collected_type;
use super::ResolutionContext;

/// Map a React builtin type name to the appropriate `PropType`.
pub(super) fn react_type_to_prop_type(
    name: &str,
    args: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    // Strip "React." prefix for matching.
    let strip = name.strip_prefix("React.").unwrap_or(name);

    match strip {
        // React node types.
        "ReactNode" | "ReactElement" | "JSX.Element" | "ReactPortal" | "ReactFragment" | "ReactChild" => {
            PropType::ReactNode
        }

        // CSS properties.
        "CSSProperties" | "CSSObject" => PropType::CssProperties,

        // Ref types.
        "Ref" | "RefObject" | "ForwardedRef" | "MutableRefObject" | "RefCallback" | "LegacyRef" => {
            let element = args.first().map(|a| a.to_raw_string());
            PropType::Ref { element }
        }

        // ElementType — component-as-prop.
        "ElementType" => PropType::ElementType,

        // FC / FunctionComponent — return as Named.
        "FC"
        | "FunctionComponent"
        | "VFC"
        | "VoidFunctionComponent"
        | "ComponentType"
        | "ForwardRefExoticComponent" => {
            let resolved_args: Vec<PropType> =
                args.iter().map(|a| resolve_collected_type(a, consuming_file, ctx, state, depth + 1)).collect();
            PropType::Named { name: name.into(), args: resolved_args }
        }

        // ComponentPropsWithoutRef<'button'> or ComponentPropsWithoutRef<typeof X>.
        "ComponentPropsWithoutRef" | "ComponentProps" | "ComponentPropsWithRef" => {
            if let Some(first) = args.first() {
                match first {
                    CollectedType::StringLiteral(el) => {
                        PropType::HtmlAttributes { element: el.to_lowercase().to_string(), omitted: vec![] }
                    }
                    other => PropType::Named {
                        name: name.into(),
                        args: vec![resolve_collected_type(other, consuming_file, ctx, state, depth + 1)],
                    },
                }
            } else {
                PropType::Any
            }
        }

        // PropsWithChildren / PropsWithRef — resolve inner type.
        "PropsWithChildren" | "PropsWithRef" => {
            if let Some(first) = args.first() {
                resolve_collected_type(first, consuming_file, ctx, state, depth + 1)
            } else {
                PropType::Any
            }
        }

        // ElementRef.
        "ElementRef" => PropType::Ref { element: None },

        // Context / Consumer / Provider — surface as Named.
        "Context" | "Consumer" | "Provider" | "RefAttributes" => {
            let resolved_args: Vec<PropType> =
                args.iter().map(|a| resolve_collected_type(a, consuming_file, ctx, state, depth + 1)).collect();
            PropType::Named { name: name.into(), args: resolved_args }
        }

        // Default — surface as Named with resolved args.
        _ => {
            let resolved_args: Vec<PropType> =
                args.iter().map(|a| resolve_collected_type(a, consuming_file, ctx, state, depth + 1)).collect();
            PropType::Named { name: name.into(), args: resolved_args }
        }
    }
}

/// Best-effort path to the @types/react .d.ts file for RDT propFilter compat.
/// Falls back to a synthetic path if @types/react is not installed.
pub(super) fn resolve_react_types_file(from_file: &Utf8Path, ctx: &ResolutionContext) -> String {
    let dir = from_file.parent().map_or("", Utf8Path::as_str);
    if let Some(found) = ctx.react_types_files.get(dir) {
        return found.clone();
    }
    let found = resolve_package_types_file(&ctx.oxc_resolver, &ctx.types_first_resolver, from_file, "react")
        .unwrap_or_else(|| "node_modules/@types/react/index.d.ts".to_owned());
    ctx.react_types_files.insert(Box::from(dir), found.clone());
    found
}

/// Resolve `package_name` to its declaration file the way TypeScript does. `oxc_resolver`'s `resolve_dts` stops at the
/// first `exports` key that matches any condition, even one pointing at JavaScript, so two retries follow: with only
/// the `types` conditions (`types_first`), for a package that lists `"import"` before `"types"`; then the separate
/// `@types/<name>` package, for one with no types of its own. A declaration file that doesn't exist counts as none.
pub(super) fn resolve_package_types_file(
    resolver: &Resolver,
    types_first: &Resolver,
    importing_file: &Utf8Path,
    package_name: &str,
) -> Option<String> {
    let declaration = |resolver: &Resolver, specifier: &str| {
        let resolved = resolver.resolve_dts(importing_file.as_std_path(), specifier).ok()?;
        // `resolve_dts` returns an `exports` target without checking it exists (an unbuilt workspace package's `dist`).
        let is_declaration =
            [".d.ts", ".d.mts", ".d.cts"].iter().any(|ext| resolved.path().to_string_lossy().ends_with(ext));
        (is_declaration && resolved.path().is_file()).then(|| resolved.path().to_string_lossy().into_owned())
    };
    declaration(resolver, package_name)
        .or_else(|| declaration(types_first, package_name))
        .or_else(|| declaration(resolver, &format!("@types/{}", mangle_scoped_package_name(package_name))))
}

/// `@scope/name` -> `scope__name`, matching TypeScript's `@types` scoped-package
/// naming convention (e.g. `@babel/core` -> `@types/babel__core`). Unscoped names
/// pass through unchanged.
fn mangle_scoped_package_name(name: &str) -> String {
    name.strip_prefix('@').map_or_else(|| name.to_owned(), |rest| rest.replacen('/', "__", 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::PipelineOptions;
    use crate::resolver::ModuleResolvers;

    fn test_resolvers() -> ModuleResolvers {
        ModuleResolvers::new(&PipelineOptions::default())
    }

    #[test]
    fn falls_back_to_at_types_when_package_has_no_own_types() {
        let ModuleResolvers { main, types_first } = test_resolvers();
        // `react`'s package.json has an `exports` field with no "types" condition and
        // ships no .d.ts of its own — this repo's node_modules has a real `@types/react`
        // package, so this exercises the real gap end to end, not a mock.
        let from_file = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");

        let resolved = resolve_package_types_file(&main, &types_first, &from_file, "react")
            .expect("expected @types/react to resolve");

        assert!(resolved.ends_with(".d.ts"), "expected a .d.ts file, got {resolved}");
        assert!(
            resolved.contains("@types") && resolved.contains("react"),
            "expected an @types/react path, got {resolved}"
        );
    }

    #[test]
    fn resolves_own_types_directly_without_at_types_fallback() {
        let ModuleResolvers { main, types_first } = test_resolvers();
        // Resolving "@types/react" itself (rather than "react") exercises the
        // primary (non-fallback) path with no ambiguity: if it ever fell through
        // to the fallback branch, that would ask for the nonexistent
        // "@types/types__react" package and correctly return None instead — so a
        // `Some(..d.ts)` result here can only come from the primary path.
        let from_file = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");

        let resolved = resolve_package_types_file(&main, &types_first, &from_file, "@types/react")
            .expect("expected @types/react's own types to resolve directly");

        assert!(resolved.ends_with(".d.ts"), "expected a .d.ts file, got {resolved}");
    }

    #[test]
    fn mangles_scoped_package_names_for_at_types() {
        assert_eq!(mangle_scoped_package_name("react"), "react");
        assert_eq!(mangle_scoped_package_name("@babel/core"), "babel__core");
        assert_eq!(mangle_scoped_package_name("@radix-ui/react-select"), "radix-ui__react-select");
    }

    /// `node_modules/<name>` holding `files`, resolved from the project root.
    fn resolve_installed(name: &str, files: &[(&str, &str)]) -> Option<String> {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
        for (file, body) in files {
            let path = root.join("node_modules").join(name).join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let ModuleResolvers { main, types_first } = test_resolvers();
        let resolved = resolve_package_types_file(&main, &types_first, &root.join("index.ts"), name)?;
        Some(resolved.strip_prefix(root.as_str()).unwrap_or(&resolved).to_owned())
    }

    #[test]
    fn a_package_listing_import_before_types_resolves_to_its_declarations() {
        let resolved = resolve_installed(
            "icons",
            &[
                ("package.json", r#"{ "exports": { ".": { "import": "./index.mjs", "types": "./index.d.ts" } } }"#),
                ("index.mjs", "export const Icon = () => null;\n"),
                ("index.d.ts", "export interface IconProps { size?: number }\n"),
            ],
        );
        assert_eq!(resolved.as_deref(), Some("/node_modules/icons/index.d.ts"));
    }

    #[test]
    fn esm_declaration_files_resolve() {
        let resolved = resolve_installed(
            "esm-only",
            &[
                (
                    "package.json",
                    r#"{ "exports": { ".": { "types": "./dist/index.d.mts", "import": "./dist/index.mjs" } } }"#,
                ),
                ("dist/index.d.mts", "export interface Props { size?: number }\n"),
                ("dist/index.mjs", "export {};\n"),
            ],
        );
        assert_eq!(resolved.as_deref(), Some("/node_modules/esm-only/dist/index.d.mts"));
    }
}
