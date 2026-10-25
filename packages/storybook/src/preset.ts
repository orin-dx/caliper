import { fileURLToPath } from 'node:url'
import type { DocgenProviderDescriptor } from 'storybook/internal/types'
import type { CaliperStorybookOptions } from './index.js'

/** The addon's options travel to the docgen worker by structured clone, so only these plain fields go. */
function workerOptions(options: CaliperStorybookOptions): CaliperStorybookOptions {
  const {
    srcDirs,
    exclude,
    tsconfigPath,
    skipHtmlProps,
    htmlAttributes,
    cache,
    loadDependencies,
    collapseInheritedProps,
    lazy,
    warmUp,
    fallback,
  } = options
  return {
    srcDirs,
    exclude,
    tsconfigPath,
    skipHtmlProps,
    htmlAttributes,
    cache,
    loadDependencies,
    collapseInheritedProps,
    lazy,
    warmUp,
    fallback,
  }
}

/** Registered last, the provider wraps React's: it answers first and hands on what it can't document. */
export const experimental_docgenProvider = (
  existing: DocgenProviderDescriptor[] = [],
  options: CaliperStorybookOptions = {},
): DocgenProviderDescriptor[] => [
  ...existing,
  { moduleSpecifier: fileURLToPath(new URL('provider.js', import.meta.url)), options: workerOptions(options) },
]

/** Storybook runs docgen providers only with `features.docgenServer`, which this turns on unless `main.ts` sets it. */
export const isDocgenProviderEnabled = (): boolean => true
