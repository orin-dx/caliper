import type { Diagnostic } from 'caliper-docgen-napi'
import type { DiagnosticLevel } from './diagnostics.js'

export interface CaliperDocgenOptions {
  srcDirs: string[]
  exclude?: string[]
  /** Leave out the HTML attributes a component inherits from its element. Same as `htmlAttributes: 'none'`. */
  skipHtmlProps?: boolean
  /** Which HTML attributes to list: the notable ones (`'curated'`, the default), all of them or none. Wins over `skipHtmlProps`. */
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
  /** Load the type declarations your props reference from installed packages. Off by default. */
  loadDependencies?: boolean
  /**
   * Which inherited sources of more than 30 props to list as one inherited layer instead of prop by prop: those from
   * loaded dependencies (the default), every inherited source, or none.
   */
  collapseInheritedProps?: 'dependencies' | 'all' | 'none'
}
