import type { Diagnostic } from 'caliper-docgen-napi'

/** Where a component is declared and what it is called. */
export interface ComponentRef {
  filePath: string
  displayName: string
}

/** What `keyOf` and `degradedBy` read, derived from the components and diagnostics. */
export interface Lookups {
  byFile: Map<string, string[]>
  byName: Map<string, string[]>
  degradedBy: Map<string, Set<string>>
}

function push(map: Map<string, string[]>, at: string, key: string): void {
  const keys = map.get(at)
  if (keys) keys.push(key)
  else map.set(at, [key])
}

/** The distinct messages of the warnings and errors among `diagnostics`; info says nothing about the props. */
export function incompleteMessages(diagnostics: Diagnostic[]): string[] {
  return [...new Set(diagnostics.filter((diagnostic) => diagnostic.severity !== 'info').map((d) => d.message))]
}

export function diagnosticId(diagnostic: Diagnostic): string {
  return JSON.stringify(diagnostic)
}

export function buildLookups(components: Map<string, ComponentRef>, diagnostics: Iterable<Diagnostic>): Lookups {
  const byFile = new Map<string, string[]>()
  const byName = new Map<string, string[]>()
  for (const [key, ref] of components) {
    push(byFile, ref.filePath, key)
    push(byName, ref.displayName, key)
  }
  const degradedBy = new Map<string, Set<string>>()
  for (const diagnostic of diagnostics) {
    if (diagnostic.severity === 'info') continue
    for (const key of diagnostic.affects ?? []) {
      const messages = degradedBy.get(key) ?? new Set<string>()
      messages.add(diagnostic.message)
      degradedBy.set(key, messages)
    }
  }
  return { byFile, byName, degradedBy }
}
