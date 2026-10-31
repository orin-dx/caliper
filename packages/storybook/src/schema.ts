import type { Diagnostic, WireDiagnostic } from 'caliper-docgen-napi'

/** The output schema version this package reads (ADR 0017). A different major is a different shape. */
export const SUPPORTED_SCHEMA_VERSION = 4

/**
 * Returns `output` once its `schemaVersion` is one this package reads, and throws otherwise. Output without a version
 * is from a binding that predates it and has version 1's shape.
 */
export function assertSchemaVersion<T extends { schemaVersion?: number }>(output: T, what: string): T {
  const version = output.schemaVersion ?? 1
  if (version !== SUPPORTED_SCHEMA_VERSION) {
    throw new Error(
      `The caliper-docgen ${what} has schema version ${version}, and this package reads version ${SUPPORTED_SCHEMA_VERSION}. ` +
        'Update caliper-docgen-storybook and caliper-docgen-napi to the same release.',
    )
  }
  return output
}

/** Resolves each diagnostic's `affects` list id against `nameLists`. */
export function expandDiagnostics(diagnostics: WireDiagnostic[], nameLists: Record<string, string[]>): Diagnostic[] {
  return diagnostics.map(({ affects, ...rest }) => {
    if (affects === undefined) return rest
    const names = nameLists[affects]
    if (!names) throw new Error(`The caliper-docgen output refers to name list ${affects}, which its nameLists lacks.`)
    return { ...rest, affects: names }
  })
}
