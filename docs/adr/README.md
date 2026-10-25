# Architecture Decision Records

Short, dated records of _architectural_ decisions — ones that shape system structure, boundaries, or contracts other code depends on. Not a changelog, not a design doc, not a place to explain what the code does — comments and `ARCHITECTURE.md` already do that.

## Index

One line per ADR, with what it guards. Add a line when you add an ADR.

- [1. Parse with OXC instead of the TypeScript compiler](0001-oxc-over-typescript-compiler.md) — do not add a type-checking pass to the default path. The opt-in backend is 0006.
- [2. Hand-write Serialize/Deserialize for PropType instead of deriving it](0002-manual-serialize-for-prop-type.md) — do not derive them on `PropType` or `CollectedType`.
- [3. Cache the DTS parse cache with positional (not named) MessagePack](0003-positional-msgpack-cache-encoding.md) — do not reorder, insert or remove `SourceData` fields without bumping `CACHE_SCHEMA_VERSION`.
- [4. Defer full type-checker integration](0004-defer-type-checker-integration.md) — superseded by 0006.
- [5. Contain panics at a single, per-item boundary](0005-panic-containment-boundary.md) — route anything reachable from a rayon `.map()`, a `DocgenPlugin` hook or a NAPI entry point through `contain_panic`.
- [6. Run the TypeScript backend as a second resolver](0006-semantic-backend-as-second-resolver.md) — do not patch `Opaque` leaves, give `crates/core` a compiler process or I/O, or route the backend through NAPI.

- [7. Let the backend discover components, always compile it in, and keep structural as the default](0007-backend-discovers-components-and-compiles-in.md) — do not feature-gate the backend crate, run it by default, or let it replace a structural entry on a clash.
- [8. Discard the parse cache whenever the build that wrote it differs](0008-parse-cache-keyed-by-build.md) — do not load cache entries without checking the payload's header against the running build.
- [9. Write each distinct prop once in the canonical output](0009-canonical-output-interns-props.md) — do not inline props in canonical JSON or derive prop ids from position.
- [10. Load dependency declarations by name, before resolution, into a shared layer](0010-load-dependency-declarations-by-name.md) — do not load dependency files by import closure, resolve against them lazily during resolution, or emit their components or enums. Its precedence of shortcuts over dependency declarations is superseded by 0014.
- [11. Find dependency declarations through a per-package export index, not the parse cache](0011-per-package-export-index.md) — do not put dependency files in the `DtsCache` or reuse an index whose package's `package.json` changed.
- [12. Collapse large inherited dependency sources, as Storybook does](0012-collapse-large-dependency-sources.md) — do not resolve a collapsed source's props, or collapse a component's own props.
- [13. Read React's attribute declarations in curated mode, in an index of their own](0013-curated-mode-reads-react-attributes-apart.md) — do not merge React into the project index outside Full mode, or make the React index a lookup fallback.
- [14. A loaded declaration wins over a known shortcut; shortcuts are fallbacks](0014-declarations-before-shortcuts.md) — do not check a shortcut before a declaration, or add one without measuring it against the declaration on consumer code.
- [15. A member read through React's types reads React's declaration and what it imports](0015-member-reads-of-react-types-read-their-declarations.md) — do not answer an indexed access from a table before its declaration, or add React's own files to the dependency layer. Narrows 0013's "only HTML-attribute lookups read the React index" and 0010's "the loader doesn't read React".
- [16. Resolve components on request, and load dependencies for the names their resolution reaches](0016-resolve-components-on-request.md) — do not make a lazy answer differ from batch extraction's for the same key, memoize a resolution that found a dependency name missing, or expose a parser type from the session API. Extends 0010's loading from "before resolution" to "before the resolution that answers".
- [17. Version the canonical JSON, and make readers reject a version they don't read](0017-version-the-canonical-json.md) — do not change the canonical JSON's shape without bumping `SCHEMA_VERSION`, or let a reader accept a version it was not written for.
- [18. Write prop sets and name lists once in the canonical output](0018-canonical-output-interns-prop-sets-and-name-lists.md) — do not write a component's props, an inheritance layer's `omitted` or a diagnostic's `affects` inline in canonical JSON. Extends 0009's per-prop table to the lists that repeat across components; `SCHEMA_VERSION` 2.

Changing any of these needs a new ADR.

## When to write one

Two gates, both required:

1. **Architectural.** It affects one of: system structure (module/crate boundaries, how components interact), a dependency or technology choice that ripples across the codebase, a data format or contract other code depends on, or a cross-cutting constraint (performance model, correctness discipline, security model).
2. **Expensive to reverse and not obvious from the code.** If it gets reverted in a year, what breaks, and would anyone know why?

If either answer is no, it's not an ADR.

Good candidates:

- Choosing OXC over the TypeScript compiler — a technology choice that shapes the whole extraction pipeline
- A serialization format with a hidden constraint (msgpack's positional encoding) — a data contract other code depends on
- Dropping a dependency, or an approach that was tried and abandoned

Not ADR material:

- A single function's algorithm choice, confined to that function — that's a code comment
- A bug fix — commit message, and if it's the kind of thing that could recur, a line in `rdt-coverage.md`
- A behavior-preserving refactor — commit message is enough
- A naming or style convention — that's `AGENTS.md` or a lint rule
- Anything already obvious from the code or from `AGENTS.md`'s non-negotiables
- Per-user preferences, workflow notes and tool quirks — that's agent memory

## Format

One file per decision: `docs/adr/NNNN-short-title.md`, numbered sequentially, never reused. Copy `0000-template.md` to start.

- **Never edit history.** If a decision changes, write a new ADR and mark the old one's status `Superseded by 000X`. Don't rewrite the old file to look like you knew all along.
- **Status** is one of: `Accepted`, `Superseded by NNNN`, `Deprecated`.
- Keep it short. If the technical depth doesn't fit on a screen, split it into its own doc and link it — see `0006-semantic-backend-as-second-resolver.md` pointing at `type-checker-integration.md` instead of inlining it.

## Writing it

Write like you're explaining it to the next engineer over their shoulder, not presenting it to a committee. Don't hard-wrap paragraphs at a fixed column — one line per paragraph, let the editor soft-wrap. Every other doc in this repo already does this; hard-wrapping just makes future diffs noisy when a sentence gets edited.

**Cut the filler.** If a sentence still means the same thing without a word, delete the word.

| Instead of | Write |
| --- | --- |
| "It's worth noting that this decision was made in order to leverage OXC's robust parsing capabilities" | "OXC parses in parallel with no type-checking pass" |
| "This represents a significant improvement in performance" | "32ms instead of several seconds" |
| "We decided to essentially utilize a positional encoding approach" | "We use positional encoding" |
| "In conclusion, this architecture provides a solid foundation going forward" | (delete — you already said the thing) |

**Banned by default:** _leverage, utilize, robust, seamless, cutting-edge, essentially, basically, in order to, it's worth noting, it is important to note, at the end of the day._ If one is genuinely the precise technical word for something, fine — but check first.

**Concrete beats vague.** Name the file, the number, the benchmark. "Slow" is not a fact; "several seconds on a mid-size design system" is.

**Active voice, plain structure.** "We chose X because Y" beats "X was chosen due to Y considerations." Sentence-case headers, not Title Case.

**Show the trade-off, not just the win.** A decision with no downside isn't a decision, it's a fact. Say what it costs.

**Structure isn't fixed.** A paragraph, a paragraph plus bullets, a short table — whatever makes it scannable in one pass. `0006`'s Decision section is an intro line plus three bullets because it's three real choices; don't force that into one paragraph, and don't add sections or padding a one-line decision doesn't need.

## What good looks like

Read `0002-manual-serialize-for-prop-type.md` — a real bug behind it, a real number (`recursion_limit = 2048`), a real consequence, four sections, no throat-clearing.
