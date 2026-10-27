# 0013. Read React's attribute declarations in curated mode, in an index of their own

**Status:** Accepted **Date:** 2026-10-23

## Context

When two members of an intersection declare the same prop, TypeScript intersects both types (#122). An HTML layer such as `ComponentProps<"div">` lists none of its element's attributes, yet declares them all, so a library prop named like a DOM attribute (recharts' `onTimeUpdate`) has the attribute's type as well. Only Full mode merged `@types/react`, so curated mode, which the CLI, the Vite plugin and Storybook use by default, showed the library's type alone. Merging React into the project index, as Full mode does, would also resolve every other React name in curated output (`ReactNode`, `CSSProperties`, the handler aliases) against the real declarations and change far more than these collisions.

## Decision

In curated mode, the cold run parses `@types/react` (through the parse cache) into its own `NamedTypeIndex`, held by the revision and passed to `ResolutionContext::with_react_types`. Only the HTML-layer attribute lookup (`resolver/html.rs`, via `ResolutionContext::react_index`) reads it. It is not a fallback of the project index, so no other lookup sees React's declarations. Full mode keeps merging React into the project index; `none` mode and the dependency loader (0010) don't read React at all.

## Consequences

- A prop that collides with an attribute of the layer's element has both types in curated mode too, with both declarations listed. The attribute keeps its alias name (`ReactEventHandler<HTMLDivElement>`), because nothing else in curated mode expands React's aliases.
- One more declaration file per run: `dtsFilesParsed` rises by one in every fixture snapshot. Wall time on the five benchmark corpora is unchanged within run-to-run noise.
- On those corpora with default flags, 7 shadcn props change (`ChartContainer.children` now intersects React's `children`); nothing else does.
- Without `@types/react`, curated mode behaves as before and reports nothing, since it never required it.
