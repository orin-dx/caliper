import type { Diagnostic } from 'caliper-docgen-napi'
import type { DiagnosticLevel } from './diagnostics.js'

export interface CaliperDocgenOptions {
  srcDirs: string[]
  exclude?: string[]
  /** Leave out the HTML attributes a component inherits from its element. Same as `htmlAttributes: 'none'`. */
  skipHtmlProps?: boolean
  /** Which HTML attributes to list: all of them (`'full'`, the default), a compact subset (`'curated'`: handlers, `aria-*`, `ref`, common attributes) or none. Wins over `skipHtmlProps`. */
  htmlAttributes?: 'curated' | 'full' | 'none'
  /**
   * The least severe extraction diagnostic to log, once each: `'error'`, `'warn'` (the default), `'info'`, or
   * `'silent'` to log none.
   */
  logLevel?: DiagnosticLevel
  /** Called once for every diagnostic, whatever `logLevel` is. */
  onDiagnostic?: (diagnostic: Diagnostic) => void
  /** Assign each component its react-docgen-typescript docs as `__docgenInfo`, for Storybook without a docgen server. */
  injectDocgenInfo?: boolean
  /**
   * Read and write the on-disk caches under `node_modules/.cache`. Default: `true`; `false` reads and writes nothing
   * there and parses everything every run. The output is the same.
   */
  cache?: boolean
  /** Load the type declarations your props reference from installed packages. On by default; `false` skips it. */
  loadDependencies?: boolean
  /**
   * Which inherited sources to list as one inherited layer of names instead of prop by prop: large ambient ones (DOM
   * and ARIA attributes, style systems) from loaded dependencies (the default), those and project sources of more than
   * 30 props, or none. A dependency's own API is listed unless it declares more than 200 props.
   */
  collapseInheritedProps?: 'dependencies' | 'all' | 'none'
}
