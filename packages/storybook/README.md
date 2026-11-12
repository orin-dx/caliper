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
| `exclude` |  | Glob patterns to skip, gitignore-style and relative to each source directory: `**/*.fixture.tsx`, `legacy/`. A bare name such as `Button` matches only a file or folder named exactly that. |
| `tsconfigPath` |  | tsconfig for path aliases. |
| `skipHtmlProps` | `false` | Leave out the HTML attributes a component inherits from its element. |
| `htmlAttributes` | `full` | `full` lists all of the HTML attributes, `curated` a compact subset (handlers, `aria-*`, `ref`, common attributes), `none` none. Wins over `skipHtmlProps`. |
| `cache` | `true` | Read and write the on-disk caches under `node_modules/.cache`. `false` reads and writes nothing there and parses everything every run; the output is the same. |
| `loadDependencies` | `true` | Read the type declarations your props reference from installed packages, so library-typed props resolve instead of degrading. `false` skips the work. |
| `collapseInheritedProps` | `none` | `none` lists every inherited prop. `dependencies` lists large ambient dependency sources (DOM and ARIA attributes, style systems) as one layer of names; `all` also does so for project sources of more than 30 props. |
| `lazy` | `true` | Resolve a component when a story asks for it. `false` resolves every component at start; the answers are the same. |
| `fillAfter` | `0` | With `lazy`, once this many distinct components were asked for, resolve the rest in the background in one parallel pass. It lowers the median story latency of a project read end to end (Spectrum's 50 stories: 19 ms to 5 ms) but not the total time, and raises peak memory by 170 to 420 MB, so it is off. `16` is a reasonable value to try. |
| `warmUp` | `false` | With `lazy`, resolve a few components in the background after start. Off: it did not make the first story faster in measurement. |
| `prefetchStories` | `1` | When `storybook dev` starts, document the components of this many first stories through Storybook's docgen service before a browser asks, one after another. That starts the session (and the checker), and stores the opening story's answer where the browser reads it: with `semanticTypes: 'on-demand'`, Chakra's `Button` had the checker's props in the browser before it asked, where they arrived 0.7 to 2.9 s after its request without. The browser still asks the provider again, which answers from the session. A story opened while a prefetch runs waits for it, so a larger number only pays off when the browser opens later than the components take. `0` waits for the browser. |
| `semanticTypes` | `'off'` | Experimental. `'on-demand'` keeps the built-in TypeScript 7.1 checker warm and asks it for the component a story shows, so props only the type checker can see appear (shadcn's components gain more than twice as many). A story waits up to `semanticWaitMs` for it and otherwise shows the usual props; the checker's answer is kept for the next request. The checker answers the stories you open before `warmUp` and `fillAfter` work, and a story whose component it is already answering waits for that answer instead of asking again. Nothing to install: the checker is part of the addon's native binding. |
| `tscPath` | the built-in checker | A checker binary for `semanticTypes: 'on-demand'` to run instead: `typescript@7.1`'s `tsc`, or `tsrs`. |
| `semanticWaitMs` | `150` | How many milliseconds a story waits for the checker before it takes the usual props. |
| `semanticWaitEmptyMs` | `2000` | How many milliseconds a story waits for the checker when caliper-docgen's own pass lists no props for its component (for example `ComponentProps<typeof Button>` of a library it cannot read), since there is nothing to show before the checker answers. A value below `semanticWaitMs` gives such a story `semanticWaitMs`. |
| `fallback` | `degraded` | `degraded`: a component whose extraction reported a warning or error goes to React's docgen; a prop it could only report as opaque (an `info` diagnostic) does not, and shows its type text with an `object` control. Once the checker answers for a component, a warning about what its answer replaces (a component caliper-docgen could not find, a type it could not resolve) no longer counts; one about what the checker could not read either still does. `never`: always answer. |

A story whose component caliper-docgen can't find goes to React's docgen, which stays registered underneath. Each component handed on is logged once as a warning with the reason (the extraction diagnostics that affect it).

The defaults favour completeness over Storybook's: caliper-docgen is fast enough to read dependency types and list inherited props that a checker-backed docgen leaves out.

## Differences from React's docgen

React's docgen in Storybook 11 asks the TypeScript checker for each component. On the same components, the argTypes differ in two ways:

- A string default keeps caliper-docgen's quotes: `"md"` where React's docgen shows the source text, `'md'`.
- Type text is printed as written: `MouseEvent<HTMLButtonElement>` where the checker prints `MouseEvent<HTMLButtonElement, MouseEvent>`, and `{ date?: string }` where it prints `{ date?: string | undefined; }`.
