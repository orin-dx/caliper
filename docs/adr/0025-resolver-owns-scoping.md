# 25. The resolver owns scoping; the extractor records where a declaration sits

**Status:** Accepted **Date:** 2026-11-06

## Context

Which declaration a name refers to is decided in more than one place, and none of them is the resolver.

- The extractor keys every declaration by `scoped_key` (`extractor/mod.rs`): the file, the enclosing namespaces and the bare name. A function body or block adds no part to the key. `local_depth` (`extractor/visit.rs`, field in `extractor/mod.rs`) is read by one collector, `visit_ts_interface_declaration`, to decide whether a declaration may merge with, replace or be skipped for a module-level one. `visit_ts_type_alias_declaration` and the enum collectors in `extractor/interface.rs` insert under the same key without asking, so a local one replaces a module-level one of that name.
- `docs/edge-cases.md` lists what this leaves open: a local interface, enum or class is not isolated from a module-level one of the same name, two local ones overwrite each other, and an interface directly in a `switch` case counts as module-level, because `visit_switch_statement` raises `block_depth` but not `local_depth`.
- The resolver's `TypeScope` (`types/scope.rs`, held in `ResolveState.scope` in `types/global.rs`) binds type parameters only. Its doc comment says declarations sit at module level, so the resolver has no notion of a declaration's scope to consult.
- `ImportRef.is_type_only` (`import_map.rs`) is copied from the import and carries `#[allow(dead_code)]`; only that file's tests read it. The one reader of `ImportBinding.is_type_only` is `promote_reexported_imports` in `extractor/mod.rs`, which folds it into a re-export, so the extractor decides a status the resolver never sees.

Each scope fix so far was a rule added to one collector, and each collector got its own rule. The next kind of declaration or block repeats the question.

## Decision

The resolver owns name resolution and scoping. The extractor records every declaration and never decides which one a name refers to.

- Every collected declaration (interface, alias, enum, class) carries its enclosing scope identity, assigned by the extractor from where the declaration sits: module, namespace, function body, block, or `switch` case. The extractor stops dropping, merging or replacing declarations by name. Two declarations that differ in scope are two entries.
- The resolver looks a name up from the reference's scope outward, with `TypeScope` or a sibling type beside it, so type parameters and declarations shadow under one rule. Merging of same-scope declarations (interfaces, enums) moves with it.
- `is_type_only` is recorded once, by the extractor, on the import. The resolver reads it from `ImportRef`; nothing re-derives it, and the `dead_code` allowance goes.

## Consequences

- `local_depth`, `module_interfaces` and the module-level checks in `visit_ts_interface_declaration` go away. The `docs/edge-cases.md` entries on local declarations and the `switch` case are removed when the branch that closes them lands.
- Collected shapes gain a scope identity, which changes `SourceData`. That is a positional MessagePack change (ADR 0003): `CACHE_SCHEMA_VERSION` (`cache.rs`) moves with it and `cache/golden.rs` fails until it does. A change to the canonical JSON would follow ADR 0017.
- Lookup by scope costs a walk outward per reference where it is a map hit today, and entries per name grow. Each resolver branch needs the Chakra plus dependencies run for peak RSS and wall time against `main`, with the addon's real configuration (collapse `none`), not only a fixture.
- Follow-up branches, each small and independent of the later ones, in this order:
  1. Add the scope identity to collected declarations and populate it, with no reader. Bumps `CACHE_SCHEMA_VERSION`.
  2. Resolve interfaces by scope; drop `local_depth` and `module_interfaces`. Closes the interface half of the edge case.
  3. Give a `switch` case its own scope in the walk, so a declaration directly in a case is local.
  4. Do the same for aliases and enums, then classes.
  5. Read `ImportRef.is_type_only` in the resolver where type-only status matters, and remove the `dead_code` allowance. If nothing there needs it, remove the field instead.
- Unsolved: declaration merging across scopes that TypeScript allows (a `namespace` merging with an interface or class), global and ambient augmentation, and `typeof` and value-space shadowing are out of this decision. The structural pass stays an approximation, and the opt-in backend of ADR 0006 is still the answer where it must match the compiler.
- Risk: until branch 2 lands the extractor and resolver disagree about scope. Branch 1 reads nothing, so it changes no output; the later branches change output only where the edge case applied.

## Alternatives considered

- Keep patching in the extractor, as `local_depth` did for interfaces. Each collector still decides alone, the aliases, enums and classes need a copy of the rule, and a reference is resolved in the resolver on a key the extractor has already collapsed. The information a scope-aware lookup needs is gone by then.
- Move all scoping into the extractor, resolving each reference to a unique key while it walks. The extractor would then answer which declaration a name means, but it sees one file; imports, re-exports, dependency declarations and type arguments are only known to the resolver, so it would have to resolve twice or leave the cross-file half unscoped.
