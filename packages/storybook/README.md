# caliper-docgen-storybook

A Storybook 11 addon that answers Storybook's docgen requests from a caliper-docgen session, so the props table and controls come from caliper-docgen instead of the TypeScript checker.

```bash
npm install --save-dev caliper-docgen-storybook
```

```ts
// .storybook/main.ts
import type { StorybookConfig } from '@storybook/react-vite'

const config: StorybookConfig = {
  framework: '@storybook/react-vite',
  stories: ['../src/**/*.stories.tsx'],
  addons: [{ name: 'caliper-docgen-storybook', options: { srcDirs: ['src'] } }],
}
export default config
```

The addon registers a docgen provider (`experimental_docgenProvider`) and turns on `features.docgenServer`. For each story file, the provider:

1. Resolves `meta.component` (and `meta.subcomponents`) to a file and export, the way Storybook does.
2. Finds that component in the session. A component imported through a barrel file is matched by its exported name.
3. Converts its react-docgen-typescript-shaped docs to argTypes with `@storybook/react`'s own converter, after the same filtering React's docgen applies.

Before answering, the provider compares every source file's mtime and size with the last look and applies the changed ones, so an answer always includes the edits saved before the request.

## Options

| Option | Default | Meaning |
| --- | --- | --- |
| `srcDirs` | `['src']` | Directories to extract from, relative to Storybook's working directory. |
| `exclude` |  | Glob patterns to skip. |
| `tsconfigPath` |  | tsconfig for path aliases. |
| `skipHtmlProps` | `false` | Leave out the HTML attributes a component inherits from its element. |
| `htmlAttributes` | `full` | `full` lists all of the HTML attributes, `curated` a compact subset (handlers, `aria-*`, `ref`, common attributes), `none` none. Wins over `skipHtmlProps`. |
| `cache` | `true` | Read and write the on-disk caches under `node_modules/.cache`. `false` reads and writes nothing there and parses everything every run; the output is the same. |
| `loadDependencies` | `true` | Read the type declarations your props reference from installed packages, so library-typed props resolve instead of degrading. `false` skips the work. |
| `collapseInheritedProps` | `none` | `none` lists every inherited prop. `dependencies` or `all` list inherited sources of more than 30 props as one layer. |
| `lazy` | `true` | Resolve a component when a story asks for it. `false` resolves every component at start; the answers are the same. |
| `fillAfter` | `0` | With `lazy`, once this many distinct components were asked for, resolve the rest in the background in one parallel pass. It lowers the median story latency of a project read end to end (Spectrum's 50 stories: 19 ms to 5 ms) but not the total time, and raises peak memory by 170 to 420 MB, so it is off. `16` is a reasonable value to try. |
| `warmUp` | `false` | With `lazy`, resolve a few components in the background after start. Off: it did not make the first story faster in measurement. |
| `semanticTypes` | `'off'` | Experimental. `'on-demand'` keeps one TypeScript 7.1 `tsc` warm and asks it for the component a story shows, so props only the type checker can see appear (shadcn's components gain more than twice as many). A story waits up to `semanticWaitMs` for it and otherwise shows the usual props; the checker's answer is kept for the next request. Needs `typescript@beta` installed, or `tscPath`. |
| `tscPath` | the nearest `node_modules/typescript/bin/tsc` | The `tsc` binary `semanticTypes: 'on-demand'` runs. |
| `semanticWaitMs` | `150` | How many milliseconds a story waits for the checker before it takes the usual props. |
| `fallback` | `degraded` | `degraded`: a component whose extraction reported a warning or error goes to React's docgen. `never`: always answer. |

A story whose component caliper-docgen can't find goes to React's docgen, which stays registered underneath. Each component handed on is logged once as a warning with the reason (the extraction diagnostics that affect it).

The defaults favour completeness over Storybook's: caliper-docgen is fast enough to read dependency types and list inherited props that a checker-backed docgen leaves out.

## Differences from React's docgen

React's docgen in Storybook 11 asks the TypeScript checker for each component. On the same components, the argTypes differ in two ways:

- A string default keeps caliper-docgen's quotes: `"md"` where React's docgen shows the source text, `'md'`.
- Type text is printed as written: `MouseEvent<HTMLButtonElement>` where the checker prints `MouseEvent<HTMLButtonElement, MouseEvent>`, and `{ date?: string }` where it prints `{ date?: string | undefined; }`.
