# 0012. Collapse large inherited dependency sources, as Storybook does

**Status:** Accepted **Date:** 2026-10-23

## Context

With dependency declarations loaded (0010), a component inherits every prop of the package types it extends: a Radix primitive's handful, but also hundreds of style props. In chakra's own repository the prop count went from 560 to 398,910 (about 665 per component). Storybook 11's react-component-meta resolves everything and then hides a source from `node_modules` (or a `.d.ts`) that contributes more than 30 props, and React's `Attributes`/`RefAttributes`; RDT's default filter hides every prop declared in `node_modules`. Both resolve before they filter; skipping the resolution is where the time and memory go.

## Decision

`collapseInheritedProps: "dependencies" | "all" | "none"`, default `"dependencies"`, threshold 30, applied before resolving an inherited source.

- **`"dependencies"`:** an inherited interface or alias declared in a dependency that contributes more than 30 props is not expanded. The component records it as an `InheritedLayer` with its `total_props`, and keeps only the curated notable props, as HTML attributes already work. Smaller dependency sources (`asChild`, `side`, `align`) are expanded into `props`.
- **`"all"`:** the same rule for project-declared sources too.
- **`"none"`:** expand everything, as RDT does before its filter.
- A component's own declared props are never collapsed.

## Consequences

- Default output matches what Storybook 11 shows: in an app that uses chakra or Mantine their style props collapse; in the design system's own repository, where they are project sources, they are listed.
- The design system's own repository keeps its large prop sets by default; sharing resolved props instead of copying them keeps that affordable (chakra with the experiment's loader: 2.1 s and 3.0 GB copied, 377 ms and 1.3 GB shared).
- A consumer that wants every prop sets `"none"` and pays the expansion.
