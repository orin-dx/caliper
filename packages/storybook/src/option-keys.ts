import type { CaliperStorybookOptions } from './index.js'

/**
 * Every key of the addon's options. `satisfies` makes the compiler reject a key that is missing here or that the
 * options type lacks, so the list cannot drift from `CaliperStorybookOptions`. Only `workerOptions` reads it; the
 * session options in `provider.ts` and `options.ts` still name their keys by hand, since they resolve paths and
 * apply defaults.
 */
const OPTION_KEY_SET = {
  srcDirs: true,
  exclude: true,
  tsconfigPath: true,
  skipHtmlProps: true,
  htmlAttributes: true,
  cache: true,
  loadDependencies: true,
  collapseInheritedProps: true,
  lazy: true,
  warmUp: true,
  fillAfter: true,
  semanticTypes: true,
  tscPath: true,
  semanticWaitMs: true,
  semanticWaitEmptyMs: true,
  fallback: true,
} satisfies Record<keyof CaliperStorybookOptions, true>

export const OPTION_KEYS = Object.keys(OPTION_KEY_SET) as (keyof CaliperStorybookOptions)[]
