/** The output schema version this package reads (ADR 0017). A different major is a different shape. */
export const SUPPORTED_SCHEMA_VERSION = 1

/**
 * Returns `output` once its `schemaVersion` is one this package reads, and throws otherwise. Output without a version
 * is from a binding that predates it and has version 1's shape.
 */
export function assertSchemaVersion<T extends { schemaVersion?: number }>(output: T, what: string): T {
  const version = output.schemaVersion ?? 1
  if (version !== SUPPORTED_SCHEMA_VERSION) {
    throw new Error(
      `The caliper-docgen ${what} has schema version ${version}, and this package reads version ${SUPPORTED_SCHEMA_VERSION}. ` +
        'Update caliper-docgen-vite-plugin and caliper-docgen-napi to the same release.',
    )
  }
  return output
}
