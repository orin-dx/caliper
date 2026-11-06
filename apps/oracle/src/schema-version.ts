/** The canonical JSON schema version the oracle reads (ADR 0017). A Rust test compares it with `SCHEMA_VERSION`. */
export const SUPPORTED_SCHEMA_VERSION = 4

/** Returns `output` once its `schemaVersion` is the supported one and throws otherwise. Output without a version predates it. */
export function assertSchemaVersion<T extends { schemaVersion?: number }>(output: T, what: string): T {
  const version = output.schemaVersion ?? 1
  if (version !== SUPPORTED_SCHEMA_VERSION) {
    throw new Error(
      `The ${what} has schema version ${version}, and the oracle reads version ${SUPPORTED_SCHEMA_VERSION}. ` +
        'Run it with a caliper-docgen binary of the same release.',
    )
  }
  return output
}
