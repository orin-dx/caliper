import type { CaliperDocgenOptions } from './options.js'

const LOG_LEVELS = ['silent', 'error', 'warn', 'info'] as const

/** Throws for an option value that is not one of its choices: a typo must not silently take another meaning. */
export function assertValidOptions(options: CaliperDocgenOptions): void {
  const { logLevel } = options
  if (logLevel !== undefined && !LOG_LEVELS.includes(logLevel)) {
    throw new Error(
      `[caliper-docgen] logLevel is '${String(logLevel)}', expected ${LOG_LEVELS.map((level) => `'${level}'`).join(', ')}`,
    )
  }
}
