---
cargo/caliper-docgen-core: minor
npm/caliper-docgen: patch
---

**Enum members with computed values are no longer typed as their own name without the type checker**

With `--no-checker`, `enum Level { Low = 1 << 2 }` typed `Level.Low` as the string `"Low"`. It is now the number `4`, and a value the structural lane cannot work out stays an unresolved `Level.Low` with a warning, never a made-up string.

- The structural lane now folds the initializers TypeScript folds: parentheses, `+ - ~`, arithmetic, shifts and bitwise operators, string concatenation, templates without substitutions, and reads of earlier members of the same enum.
- A member whose value is not known (a call, a reference to a `const`) is `null` in the `enums` output instead of its name.

**Breaking:** `EnumValue` in `caliper-docgen-core` has a new `Unknown` variant, and `EnumValue::to_display_string` returns `Option<String>`. Match `Unknown` where you match `EnumValue`.
