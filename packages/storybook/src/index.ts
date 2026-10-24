export interface CaliperStorybookOptions {
  /** Directories caliper-docgen extracts from, relative to Storybook's working directory. Default: `['src']`. */
  srcDirs?: string[]
  exclude?: string[]
  tsconfigPath?: string
  /** Leave out the HTML attributes a component inherits from its element. Same as `htmlAttributes: 'none'`. */
  skipHtmlProps?: boolean
  /**
   * Which HTML attributes a component inherits from its element to list: `'curated'` (the notable ones, the default),
   * `'full'` (all of them) or `'none'`. Wins over `skipHtmlProps`.
   */
  htmlAttributes?: 'curated' | 'full' | 'none'
  /**
   * Load the type declarations your props reference from installed packages, so props typed by a library resolve
   * instead of degrading. Default: `true`; `false` skips the extra work.
   */
  loadDependencies?: boolean
  /**
   * Which inherited sources of more than 30 props to list as one inherited layer instead of prop by prop. Default:
   * `'none'`, every inherited prop listed.
   */
  collapseInheritedProps?: 'dependencies' | 'all' | 'none'
  /**
   * Resolve a component when a story asks for it instead of every component when Storybook starts. Default: `true`.
   * The props are the same; the session starts sooner and holds only the components your stories use.
   */
  lazy?: boolean
  /**
   * With a lazy session, resolve a few components in the background after it starts, so the first story does not
   * load the dependencies stories share. Default: `true`.
   */
  warmUp?: boolean
  /**
   * When a component's resolution reported a warning or an error, its props may be incomplete. `'degraded'` (the
   * default) hands such a component to the next docgen provider, React's own; `'never'` always answers.
   */
  fallback?: 'degraded' | 'never'
}

export { toArgTypes } from './arg-types.js'
export type { ExtractArgTypes } from './arg-types.js'
