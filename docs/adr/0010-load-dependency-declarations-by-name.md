# 0010. Load dependency declarations by name, before resolution, into a shared layer

**Status:** Accepted **Date:** 2026-10-23

## Context

The pipeline parses only the project's own files, `@types/react` (full HTML-attribute mode) and TypeScript's lib files. An import into `node_modules` resolves to a path, but nothing parses that file, so a prop typed by a package (`ClosableType`, `OnValueChange`, a Radix primitive's props) stays an unresolved name. With dependencies installed, unresolved types sat at 116 (antd), 323 (chakra), 586 (spectrum) and about 1,000 (shadcn), and RDT expanded what we couldn't.

Loading whole files along imports is the wrong unit. The file-level closures of the five benchmark projects hold 150–6,218 files, and the cost is the file count: about 25 µs per file, almost all of it opening the file (18 threads open only 1.7× faster than one on macOS), against 418 MB/s for parsing. A prototype that loads only the files declaring a name the project uses read 39–833 files warm instead of 150–6,218.

## Decision

Behind `PipelineOptions::load_dependencies` (off by default here; on by default since [0019](0019-load-dependencies-by-default.md)), a phase before `NamedTypeIndex::build` loads the dependency declarations the project's type positions name, transitively, and nothing else.

- **Demands, not files.** A demand is a package specifier and an imported name used in a type position (a type reference, `typeof X`, heritage, a type argument). A loaded declaration's own references become new demands. One rayon scope runs them; each declaring file is claimed once, and results are keyed by file and name, so the outcome does not depend on scheduling.
- **A separate, shared layer.** Loaded declarations live in a `DependencyLayer` behind an `Arc`, with its own type index and import map. Project lookups fall back to it; keys are file paths, so the layers never collide. Watch revisions share it; an edit that adds a demand gets a new layer version with the new declarations added, never a copy of the project's data.
- **Precedence:** project declaration, then a known shortcut (`VariantProps`, `SxProps`, `HTMLChakraProps`, …), then a dependency declaration, then a builtin. The shortcuts exist because the real declarations are conditional types the structural resolver can't evaluate; a loaded declaration must not replace them.
- **Values stay out of the output.** A dependency's components go into a value index that `ComponentProps<typeof X>` and `typeof Pkg.Root` consult; they are never emitted. Dependency enums never become public output enums or feed the bare-name enum index.
- **Every unresolved demand is reported** on the component whose resolution reached it: a package with no types, an unresolvable specifier, a name the package doesn't export.

## Consequences

- In the experiment that proved this design (`exp/dependency-types`), unresolved types fell to 8 (mantine), 26 (chakra), 82 (shadcn), 58 (antd) and 172 (spectrum), for 2–29 ms of loading on a warm index.
- Props that reach large shared types grow sharply (chakra 560 → 398,910 in its own repo), which is why 0012 collapses large dependency sources and why resolved props are shared, not copied (#105).
- A watch edit that adds a package import costs one index lookup and a few files (under 1 ms in the prototype); `node_modules`, `package.json` and tsconfig changes still need `rebuild()`.
- `declare module` augmentations, `declare global` and `node:*` ambient modules are not loaded yet; a name only they provide stays unresolved, with a diagnostic.
- The TypeScript backend (0006, 0007) gets fewer incomplete components to handle; both now report parents inside `node_modules`.
