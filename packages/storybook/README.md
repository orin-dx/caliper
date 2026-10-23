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
| `fallback` | `degraded` | `degraded`: a component whose extraction reported a warning or error goes to React's docgen. `never`: always answer. |

A story whose component caliper-docgen can't find goes to React's docgen, which stays registered underneath.

## Differences from React's docgen

React's docgen in Storybook 11 asks the TypeScript checker for each component. On the same components, the argTypes differ in two ways:

- A string default keeps caliper-docgen's quotes: `"md"` where React's docgen shows the source text, `'md'`.
- Type text is printed as written: `MouseEvent<HTMLButtonElement>` where the checker prints `MouseEvent<HTMLButtonElement, MouseEvent>`, and `{ date?: string }` where it prints `{ date?: string | undefined; }`.
