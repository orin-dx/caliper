import { RANK } from './diagnostics.js'
import type { CaliperDocgenOptions } from './options.js'

/** Throws for an option value that is not one of its choices: a typo must not silently take another meaning. */
export function assertValidOptions(options: CaliperDocgenOptions): void {
  const { logLevel } = options
  if (logLevel !== undefined && !Object.hasOwn(RANK, logLevel)) {
    throw new Error(
      `[caliper-docgen] logLevel is '${String(logLevel)}', expected ${Object.keys(RANK)
        .map((level) => `'${level}'`)
        .join(', ')}`,
    )
  }
}
