# caliper-docgen-semantic

## 0.4.0

- With the experimental TypeScript checker on, a function prop keeps the optional and rest markers its parameters are written with. antd's `Progress.format` reads `(percent?: number, successPercent?: number) => ReactNode` and `ActionButton.close` reads `(...args: any[]) => void`; the checker's answer used to lose the markers, giving `(percent: undefined | number, …)` and `(args: any[]) => void`, which changes how a call site reads.
- With the experimental TypeScript checker on, a prop whose type is a name that does not resolve (an import that cannot be found, a name declared nowhere) is no longer reported as `any`. It is `opaque` and carries `degradedBy: ["unresolvableImport"]`, so a reader can tell it from a prop written as `any`.
- With the experimental TypeScript checker on, a prop whose name starts with two underscores keeps the name the source writes. Mantine's `__size`, `__vars` and `__clearable` used to come back as `___size`, `___vars` and `___clearable` (the checker keeps such names with a third underscore), so those props appeared under a name nobody wrote.
- The experimental TypeScript checker can now run on `tsrs` (`npm i tsrs`) as well as on `typescript@beta`. Caliper finds `node_modules/tsrs/bin/tsrs` when no `typescript` 7.1 is installed, and `--tsc-path` (or `tscPath`) also takes the `@tsrs/<platform>/tsrs` binary, which used to fail with "could not determine tsc's version". Only the `tsrs` build that was checked against Caliper is accepted (`0.23.0-ts7.1.0-beta`); any other `tsrs` release is refused with the usual warning until it has been checked. On Chakra, Caliper's output was identical to the one from `typescript@7.1.0-beta`. The checker is still off by default.
- The minimum supported Rust version is now 1.99, and the crates use edition 2024. Building from source needs Rust 1.99 or newer; prebuilt binaries and npm packages are unaffected, and the output is unchanged.
- Agents reading TOON output no longer lose what they need to write a valid call. A literal union such as shadcn's `size` or `variant` now lists every value instead of ending in `...(+2)`, function props keep their parameter names (`(value: string, index?: number) => void`), a prop's `@deprecated`, `@see` and `@since` notes appear in a `tags` column, and a discriminated union names its discriminant in a `discriminant:` line. TOON is about 12% longer on antd and shadcn; unions of types past 8 members and intersections past 4 are still cut, with the count.
- Prop types now keep more of what TypeScript says about them. Tuples keep their optional and rest elements and their labels: `[string, ...number[]]` used to print as `[string, number[][]]`, and `[head: string, ...tail: number[]]` used to come out partly unreadable. A spread of a known tuple, `[string, ...Tail]`, is flattened as TypeScript does. Bigint literals (`10n`), negative number literals (`-1`), `unique symbol`, enum members (`Size.Small`, offered to Storybook as its value `"small"`) and template literal patterns (`` `${number}px` ``) are represented instead of being marked opaque. An object type with several call signatures, a `new` signature, or a call signature beside properties keeps them all, and a recursive alias such as `type Json = string | Json[]` refers back to itself by name instead of being cut off. A getter is read as a property (`get size(): number` is `readonly size: number`), not as a method.

  The JSON output moves to schema version 5. A tuple's elements are now `{ "type", "label"?, "modifier"? }` objects, the type kinds `bigintLiteral`, `uniqueSymbol`, `enumMember`, `templateLiteral`, `callable`, `recursive` and `truncated` are new, and the `templateLiteral` reason of an `opaque` type is gone. Update `caliper-docgen`, the Vite plugin and the Storybook addon together; a mismatched install stops with a message naming both versions.

## 0.3.0

- The experimental on-demand TypeScript 7.1 checker now works with any 7.1 release (beta, rc or stable) instead of only `7.1.0-beta`, replaces its `tsc` process when it grows past 768 MiB between requests, and no longer makes closing a session wait for a request in progress. For library users: `PINNED_VERSIONS` is now `SUPPORTED_VERSIONS`, and `expected_versions` entries may be a `major.minor` series such as `"7.1"`.

## 0.2.0

- Storybook can now ask TypeScript 7.1 for the exact component a story shows, so props that only the type checker can see appear in its controls (shadcn's components gain more than twice as many). It is experimental and off by default: set `semanticTypes: 'on-demand'` in the addon options and install `typescript@beta`. A story waits up to 150 ms for the checker and otherwise shows the usual props, with the checker's answer ready for the next request. Props the checker does not see stay, so `children`, `aria-*` and DOM handlers appear as `htmlAttributes` says, in both modes.

## 0.1.1

- Publish the TypeScript transport and semantic backend crates before the CLI. The CLI's runtime opt-in backend remains available in its crates.io distribution, and every local crate dependency now has a version requirement that Cargo can publish.

