import { resolve } from 'node:path'
import type { ComponentEntry, ComponentExport, Diagnostic } from 'caliper-docgen-napi'

/** Where a component is declared, what it is called, and how its module exports it. */
export interface ComponentRef {
  filePath: string
  displayName: string
  exports: ComponentExport[]
}

export function refOf(entry: Pick<ComponentEntry, 'filePath' | 'displayName' | 'exports'>): ComponentRef {
  return { filePath: resolve(entry.filePath), displayName: entry.displayName, exports: entry.exports }
}

/** The only key of `keys`: no key among several is the one asked for. */
export function only(keys: string[] | undefined): string | undefined {
  return keys?.length === 1 ? keys[0] : undefined
}

/** What a story's import calls a component: the name its module exports it under and the name the story gives it. */
export interface ImportName {
  exportName: string
  localName: string
}

/** Identifies one way a file exports a component: its export name, then the properties an attached sub-component is
 * reached through (`default`, `Item` for `Menu.Item = Item` beside `export default Menu`). */
export function exportKey(filePath: string, name: string, members: readonly string[] = []): string {
  return [filePath, name, ...members].join('\u0000')
}

/** What `keyOf` and `degradedBy` read, derived from the components and diagnostics. */
export interface Lookups {
  byFile: Map<string, string[]>
  byName: Map<string, string[]>
  /** The components each way of exporting names, by `exportKey`. */
  byExport: Map<string, string[]>
  degradedBy: Map<string, Set<string>>
}

function push(map: Map<string, string[]>, at: string, key: string): void {
  const keys = map.get(at)
  if (keys) keys.push(key)
  else map.set(at, [key])
}

/**
 * The distinct messages of the warnings and errors among `diagnostics`. Info does not count, opaque props included:
 * counting them would hand on most components of the measured corpora (docs/storybook-parity.md), and the answer
 * names no prop to replace.
 */
export function incompleteMessages(diagnostics: Diagnostic[]): string[] {
  return [...new Set(diagnostics.filter((diagnostic) => diagnostic.severity !== 'info').map((d) => d.message))]
}

export function diagnosticId(diagnostic: Diagnostic): string {
  return JSON.stringify(diagnostic)
}

export function buildLookups(components: Map<string, ComponentRef>, diagnostics: Iterable<Diagnostic>): Lookups {
  const byFile = new Map<string, string[]>()
  const byName = new Map<string, string[]>()
  const byExport = new Map<string, string[]>()
  for (const [key, ref] of components) {
    push(byFile, ref.filePath, key)
    push(byName, ref.displayName, key)
    for (const { name, members } of ref.exports) push(byExport, exportKey(ref.filePath, name, members), key)
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
  return { byFile, byName, byExport, degradedBy }
}
