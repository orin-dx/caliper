import type { CaliperStorybookOptions } from './index.js'

const FALLBACKS = ['degraded', 'never'] as const
const SEMANTIC_TYPES = ['off', 'on-demand'] as const

function assertOneOf(name: string, value: unknown, choices: readonly string[]): void {
  if (value === undefined || choices.includes(value as string)) return
  throw new Error(
    `[caliper-docgen] ${name} is '${String(value)}', expected ${choices.map((choice) => `'${choice}'`).join(' or ')}`,
  )
}

/** Throws for an option value that is not one of its choices: a typo must not silently take another meaning. */
export function assertValidOptions(options: Pick<CaliperStorybookOptions, 'fallback' | 'semanticTypes'>): void {
  assertOneOf('fallback', options.fallback, FALLBACKS)
  assertOneOf('semanticTypes', options.semanticTypes, SEMANTIC_TYPES)
}
