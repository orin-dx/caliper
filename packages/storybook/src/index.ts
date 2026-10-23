export interface CaliperStorybookOptions {
  /** Directories caliper-docgen extracts from, relative to Storybook's working directory. Default: `['src']`. */
  srcDirs?: string[]
  exclude?: string[]
  tsconfigPath?: string
  /** Leave out the HTML attributes a component inherits from its element. */
  skipHtmlProps?: boolean
  /**
   * When a component's resolution reported a warning or an error, its props may be incomplete. `'degraded'` (the
   * default) hands such a component to the next docgen provider, React's own; `'never'` always answers.
   */
  fallback?: 'degraded' | 'never'
}

export { toArgTypes } from './arg-types.js'
export type { ExtractArgTypes } from './arg-types.js'
