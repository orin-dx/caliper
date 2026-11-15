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
| `prefetchStories` | `1` | How many stories' components to document before the browser asks. When `storybook dev` starts, the opening story's component is documented through Storybook's docgen service: that starts the session (and the checker), and stores the answer where the browser reads it: with the checker on, Chakra's `Button` had the checker's props in the browser before it asked, where they arrived 0.7 to 2.9 s after its request without. Then, after it and after every story you open, the components of the next `prefetchStories - 1` stories in the sidebar are resolved in the background, nearest first, the checker's answers included and kept in the session's shared answer cache; the checker answers every story you open first, and a newer list replaces what is still queued. `1` (the default) prefetches only the opening story, since a wider prefetch is not measured free; `0` waits for the browser. |
| `checker` | `true` | Keep the built-in TypeScript checker warm and ask it for the component a story shows, so its props are the type checker's (shadcn's components gain more than twice as many props). A story waits up to `semanticWaitMs` for it and otherwise shows caliper-docgen's own structural props, which the checker's answer replaces in the open story when it arrives, without a reload (Storybook's docgen service is asked again, and its Controls panel and docs read it live): once per component and change to the files, only for the story on screen (one you left is updated once when you come back), and equal to what a fresh checker session answers. The answer is also kept for the next request. The checker answers the stories you open before the background prefetch, and a story whose component it is already answering waits for that answer instead of asking again. Nothing to install: the checker is part of the addon's native binding. `false` never starts it. (It replaces `semanticTypes`, which the addon now refuses.) |
| `semanticWaitMs` | `150` | How many milliseconds a story waits for the checker before it takes the usual props. |
| `semanticWaitEmptyMs` | `2000` | How many milliseconds a story waits for the checker when caliper-docgen's own pass lists no props for its component (for example `ComponentProps<typeof Button>` of a library it cannot read), since there is nothing to show before the checker answers. A value below `semanticWaitMs` gives such a story `semanticWaitMs`. |
| `fallback` | `degraded` | `degraded`: a component whose extraction reported a warning or error goes to React's docgen; a prop it could only report as opaque (an `info` diagnostic) does not, and shows its type text with an `object` control. Once the checker answers for a component, a warning about what its answer replaces (a component caliper-docgen could not find, a type it could not resolve) no longer counts; one about what the checker could not read either still does. `never`: always answer. |

A story whose component caliper-docgen can't find goes to React's docgen, which stays registered underneath. Each component handed on is logged once as a warning with the reason (the extraction diagnostics that affect it).

The defaults favour completeness over Storybook's: caliper-docgen reads dependency types and lists inherited props that a checker-backed docgen leaves out. A session that asked the checker for every component of a 938-component Chakra and MUI app peaked at about 1 GB, against 380 MB with `checker: false` (`docs/gate-measurements.md`).

## Differences from React's docgen

React's docgen in Storybook 11 asks the TypeScript checker for each component. On the same components, the argTypes differ in two ways:

- A string default keeps caliper-docgen's quotes: `"md"` where React's docgen shows the source text, `'md'`.
- Type text is printed as written: `MouseEvent<HTMLButtonElement>` where the checker prints `MouseEvent<HTMLButtonElement, MouseEvent>`, and `{ date?: string }` where it prints `{ date?: string | undefined; }`.
