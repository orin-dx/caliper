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

        // Named event handlers (e.g. MouseEventHandler).
        n if n.ends_with("EventHandler") || n.ends_with("Handler") => {
            PropType::EventHandler { event_type: name.to_owned(), param_name: None }
        }

        // Synthetic and DOM events — the type IS the event type.
        "SyntheticEvent" | "MouseEvent" | "KeyboardEvent" | "ChangeEvent" | "FocusEvent" | "FormEvent"
        | "DragEvent" | "TouchEvent" | "WheelEvent" | "AnimationEvent" | "TransitionEvent" | "ClipboardEvent"
        | "CompositionEvent" | "PointerEvent" => {
            let raw_args: Vec<String> = args.iter().map(|a| a.to_raw_string()).collect();
            let event_type =
                if raw_args.is_empty() { name.to_owned() } else { format!("{}<{}>", name, raw_args.join(", ")) };
            PropType::EventHandler { event_type, param_name: None }
        }

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
    let Some(from_dir) = from_file.parent() else {
        return "node_modules/@types/react/index.d.ts".to_owned();
    };
    resolve_package_types_file(&ctx.oxc_resolver, from_dir, "react")
        .unwrap_or_else(|| "node_modules/@types/react/index.d.ts".to_owned())
}

/// Resolve `package_name` to its real `.d.ts` file, following the same fallback
/// TypeScript's own resolver uses: if the package has no types of its own
/// (common for packages with an `exports` map but no `"types"` condition —
/// `oxc_resolver`'s `resolve_dts` stops at the first `exports` match, even a
/// plain JS one, so it never reaches its own `@types` fallback for these), retry
/// against the separate `@types/<name>` package.
pub(super) fn resolve_package_types_file(
    resolver: &Resolver,
    from_dir: &Utf8Path,
    package_name: &str,
) -> Option<String> {
    if let Ok(resolved) = resolver.resolve_dts(from_dir.as_std_path(), package_name) {
        let path = resolved.path().to_string_lossy().into_owned();
        if path.ends_with(".d.ts") || path.ends_with(".d.mts") || path.ends_with(".d.cts") {
            return Some(path);
        }
    }
    let types_specifier = format!("@types/{}", mangle_scoped_package_name(package_name));
    resolver.resolve_dts(from_dir.as_std_path(), &types_specifier).ok().map(|r| r.path().to_string_lossy().into_owned())
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
    use oxc_resolver::{ResolveOptions, Resolver};

    fn test_resolver() -> Resolver {
        Resolver::new(ResolveOptions {
            condition_names: vec!["types".into(), "import".into(), "require".into(), "default".into()],
            main_fields: vec!["types".into(), "typings".into(), "module".into(), "main".into()],
            extensions: vec![".ts".into(), ".tsx".into(), ".d.ts".into(), ".js".into()],
            ..ResolveOptions::default()
        })
    }

    #[test]
    fn falls_back_to_at_types_when_package_has_no_own_types() {
        let resolver = test_resolver();
        // `react`'s package.json has an `exports` field with no "types" condition and
        // ships no .d.ts of its own — this repo's node_modules has a real `@types/react`
        // package, so this exercises the real gap end to end, not a mock.
        let from_dir = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

        let resolved =
            resolve_package_types_file(&resolver, &from_dir, "react").expect("expected @types/react to resolve");

        assert!(resolved.ends_with(".d.ts"), "expected a .d.ts file, got {resolved}");
        assert!(
            resolved.contains("@types") && resolved.contains("react"),
            "expected an @types/react path, got {resolved}"
        );
    }

    #[test]
    fn resolves_own_types_directly_without_at_types_fallback() {
        let resolver = test_resolver();
        // Resolving "@types/react" itself (rather than "react") exercises the
        // primary (non-fallback) path with no ambiguity: if it ever fell through
        // to the fallback branch, that would ask for the nonexistent
        // "@types/types__react" package and correctly return None instead — so a
        // `Some(..d.ts)` result here can only come from the primary path.
        let from_dir = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

        let resolved = resolve_package_types_file(&resolver, &from_dir, "@types/react")
            .expect("expected @types/react's own types to resolve directly");

        assert!(resolved.ends_with(".d.ts"), "expected a .d.ts file, got {resolved}");
    }

    #[test]
    fn mangles_scoped_package_names_for_at_types() {
        assert_eq!(mangle_scoped_package_name("react"), "react");
        assert_eq!(mangle_scoped_package_name("@babel/core"), "babel__core");
        assert_eq!(mangle_scoped_package_name("@radix-ui/react-select"), "radix-ui__react-select");
    }
}
