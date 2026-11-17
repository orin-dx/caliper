import type { CaliperStorybookOptions } from './index.js'

const FALLBACKS = ['degraded', 'never'] as const

function assertOneOf(name: string, value: unknown, choices: readonly string[]): void {
  if (value === undefined || choices.includes(value as string)) return
  throw new Error(
    `[caliper-docgen] ${name} is '${String(value)}', expected ${choices.map((choice) => `'${choice}'`).join(' or ')}`,
  )
}

/**
 * Throws for an option value that is not one of its choices: a typo must not silently take another meaning. The
 * checker's old opt-in, `semanticTypes`, is named too: left unread, `semanticTypes: 'off'` would quietly run it. So is
 * `tscPath`, which named a checker binary to run instead of the built-in one: left unread, it would quietly run another.
 */
export function assertValidOptions(
  options: Pick<CaliperStorybookOptions, 'fallback' | 'checker'> & { semanticTypes?: unknown; tscPath?: unknown },
): void {
  assertOneOf('fallback', options.fallback, FALLBACKS)
  if (options.checker !== undefined && typeof options.checker !== 'boolean') {
    throw new Error(`[caliper-docgen] checker is '${String(options.checker)}', expected true or false`)
  }
  if (options.semanticTypes !== undefined) {
    throw new Error(
      "[caliper-docgen] semanticTypes was replaced by checker: the checker is on by default, and 'checker: false' turns it off",
    )
  }
  if (options.tscPath !== undefined) {
    throw new Error(
      '[caliper-docgen] tscPath was removed: the checker is built in and cannot be replaced by a binary; drop the option',
    )
  }
}
