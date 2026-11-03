// A tool's canonical JSON (Caliper's `extract --json`) as the independent reference reads it: the type it gives a prop,
// or whether a collapsed inheritance layer names the prop, which surfaces it without a type to judge.
import { readFileSync, realpathSync } from 'node:fs'
import { relative } from 'node:path'
import { caliperNorm, type Norm } from './oracle-compare.ts'
import { normComponent } from './tool-universe.ts'

interface Located {
  name: string
  component: any
}

export interface Tool {
  /** Whether the tool reports exactly one component of that name in that file. */
  has(file: string, component: string): boolean
  /** The type the tool gives `prop` of `component` in `file`, and whether a collapsed layer names it. `component` is
   * absent when the tool has no single component of that name in that file. */
  lookup(file: string, component: string, prop: string): { found: boolean; norm?: Norm; layered: boolean }
}

/** A component's props as one map of name to prop id (schema 2 lists prop-set ids, each such a map). */
export function propIds(caliper: any, component: any): Record<string, string> {
  const { props } = component
  if (!Array.isArray(props)) return props ?? {}
  return Object.assign({}, ...props.map((id: string) => caliper.propSets[id]))
}

/** The prop names a collapsed layer lists. The canonical JSON interns the list: `members` is an id into `nameLists`. */
export function layerMembers(caliper: any, layer: any): string[] {
  const { members } = layer
  if (typeof members === 'string') return caliper.nameLists?.[members] ?? []
  return members ?? []
}

export function loadTool(path: string, root: string): Tool {
  const caliper = JSON.parse(readFileSync(path, 'utf8'))
  const base = realpathSync(root)
  const byFile = new Map<string, Located[]>()
  for (const component of Object.values<any>(caliper.components)) {
    const key = relative(base, realpathSync(component.filePath))
    byFile.set(key, [...(byFile.get(key) ?? []), { name: normComponent(component.displayName), component }])
  }
  return {
    has: (file, component) => (byFile.get(file) ?? []).filter((c) => c.name === normComponent(component)).length === 1,
    lookup(file, componentName, prop) {
      const matches = (byFile.get(file) ?? []).filter((c) => c.name === normComponent(componentName))
      if (matches.length !== 1) return { found: false, layered: false }
      const [{ component }] = matches
      const id = propIds(caliper, component)[prop]
      const layered = (component.inheritance ?? []).some((layer: any) => layerMembers(caliper, layer).includes(prop))
      const type = id === undefined ? undefined : caliper.props[id]?.type
      return { found: true, norm: type === undefined ? undefined : caliperNorm(type), layered }
    },
  }
}
