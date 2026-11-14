// Validates the canonical JSON the oracle reads, so a renamed key or a dangling id stops the run with its cause
// instead of shrinking the set of props compared.

import { SUPPORTED_SCHEMA_VERSION } from './schema-version.ts'

/** Throws unless `caliper` has the keys the oracle reads, at the schema version it understands. */
export function checkCaliperShape(caliper: any): void {
  if (caliper?.schemaVersion !== SUPPORTED_SCHEMA_VERSION) {
    throw new Error(
      `caliper output has schemaVersion ${JSON.stringify(caliper?.schemaVersion)}, the oracle reads ${SUPPORTED_SCHEMA_VERSION}`,
    )
  }
  for (const key of ['components', 'propSets', 'props'] as const) {
    if (typeof caliper[key] !== 'object' || caliper[key] === null) {
      throw new Error(`caliper output has no "${key}" object`)
    }
  }
  for (const [key, component] of Object.entries<any>(caliper.components)) {
    if (!Array.isArray(component?.exports)) throw new TypeError(`caliper component ${key} has no "exports" list`)
  }
}

/** One map of prop name to prop id for a component, merged from the prop sets it lists. */
export function flattenProps(caliper: any, componentKey: string, component: any): Record<string, string> {
  if (!Array.isArray(component.props)) return component.props
  const merged: Record<string, string> = {}
  for (const id of component.props) {
    const set = caliper.propSets[id]
    if (set === undefined) throw new Error(`component ${componentKey} lists prop set ${id}, which propSets lacks`)
    Object.assign(merged, set)
  }
  return merged
}

/** Throws when a comparison compared nothing, which a gate must not report as clean. */
export function requireComparison(propsShared: number, componentsMatched: number): void {
  if (propsShared === 0) {
    throw new Error(`the comparison is empty: ${componentsMatched} components matched, 0 shared props compared`)
  }
}
