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
   * Read and write the on-disk caches under `node_modules/.cache`. Default: `true`; `false` reads and writes nothing
   * there and parses everything every run. The output is the same.
   */
  cache?: boolean
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
   * With a lazy session, resolve a few components in the background after it starts. Default: `false`: it did not
   * make the first story faster in measurement.
   */
  warmUp?: boolean
  /**
   * With a lazy session, once this many distinct components have been asked for, resolve all the rest in the
   * background in one parallel pass. It lowers the median story latency of a project read end to end (Spectrum's
   * 50 stories: 19 ms to 5 ms) but not the total time, and raises peak memory by 170 to 420 MB, so it is off by
   * default: `0`.
   */
  fillAfter?: number
  /**
   * When a component's resolution reported a warning or an error, its props may be incomplete. `'degraded'` (the
   * default) hands such a component to the next docgen provider, React's own; `'never'` always answers.
   */
  fallback?: 'degraded' | 'never'
}

export { toArgTypes } from './arg-types.js'
export type { ExtractArgTypes } from './arg-types.js'
