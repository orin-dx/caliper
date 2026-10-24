# 0014. A loaded declaration wins over a known shortcut; shortcuts are fallbacks

**Status:** Accepted **Date:** 2026-10-23

## Context

`known.rs` resolves library types by name (`HTMLChakraProps`, `SxProps`, cva's `VariantProps`, …) because the structural resolver couldn't expand their declarations. ADR 0010 kept that order for loaded dependencies: a project declaration, then a shortcut, then the dependency's own declaration. Measured against react-docgen-typescript on a consumer app with Chakra, MUI, Mantine, cva and react-aria-components installed, the loaded declarations now beat the shortcuts: Chakra's `HTMLChakraProps` gives 139 of RDT's props instead of 1, its `ButtonProps` 146 instead of 0, and Mantine's `StylesApiProps` 8 instead of 3. Only cva's `VariantProps`, which needs inference (`Parameters<C>[0]`), still needs its shortcut.

## Decision

- **Order:** a declaration, the project's or a loaded dependency's, then a known shortcut, then a React builtin (`resolver/precedence.rs`).
- **Fallback:** at the props-chain level, a declaration that evaluates to nothing (no props, no inherited layer, something left unevaluated) gives way to a shortcut of the same name, if one exists (`chain::known_fallback`). Its diagnostics are dropped with it.
- **Shortcuts kept:** only where the fallback measurably beats an unresolved name: the variant-props family, react-aria's `RenderProps`/`SlotProps`, and `HTMLChakraProps`/`HTMLArkProps`/`HTMLStyledProps` (an HTML layer when the package isn't loaded), plus React's own `PropsWithChildren`, `ComponentProps` and the like.
- **Shortcuts removed:** `SxProps`/`SystemStyleObject`/`SystemCssProperties` (printed worse text than the name itself), `OverridableStringUnion`, `ThemingProps`, `StylesApiProps` (opaque placeholders a loaded declaration beats), and the no-op `MantineColor`/`MantineSize`/`MantineRadius` arm.

## Consequences

- With `--load-dependencies`, a consumer of a component library gets the library's real props. Without it, a removed shortcut's name stays an unresolved named reference with the usual warning, as any other uninstalled package's type does; `PropType::SxProps` is no longer produced.
- A new shortcut needs the same evidence: it must beat the declaration, or stand in where the declaration can't be read, on code that uses the library.
- Library repos are unaffected: their own declarations already won (no change on the five benchmark corpora).
