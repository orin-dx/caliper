// A tool's canonical JSON (Caliper's `extract --json`) as the independent reference reads it: the type it gives a prop,
// or whether a collapsed inheritance layer names the prop, which surfaces it without a type to judge.
import { readFileSync, realpathSync } from 'node:fs'
import { relative } from 'node:path'
import { layerMembers, listedProps } from './canonical.ts'
import { caliperNorm } from './caliper-norm.ts'
import { type Norm } from './oracle-compare.ts'
import { normComponent } from './tool-universe.ts'

interface Located {
  name: string
  component: any
}

export interface Tool {
  /** Whether the tool reports exactly one component of that name in that file. */
  has(file: string, component: string): boolean
  /** What the tool says of `prop` of `component` in `file`. `found` is false when it has no single component of that name
   * there. Otherwise `norm` is the prop's type when the component lists it, as its own prop or a curated inherited one
   * (the own prop wins a clash), and `layered` says a collapsed layer names it, which surfaces it without a type. */
  lookup(file: string, component: string, prop: string): { found: boolean; norm?: Norm; layered: boolean }
}

export function loadTool(path: string, root: string): Tool {
  const caliper = JSON.parse(readFileSync(path, 'utf8'))
  const base = realpathSync(root)
  const byFile = new Map<string, Located[]>()
  for (const component of Object.values<any>(caliper.components)) {
    const key = relative(base, realpathSync(component.filePath))
    byFile.set(key, [...(byFile.get(key) ?? []), { name: normComponent(component.displayName), component }])
  }
  const single = (file: string, name: string) => {
    const matches = (byFile.get(file) ?? []).filter((c) => c.name === normComponent(name))
    return matches.length === 1 ? matches[0].component : undefined
  }
  return {
    has: (file, component) => single(file, component) !== undefined,
    lookup(file, componentName, prop) {
      const component = single(file, componentName)
      if (component === undefined) return { found: false, layered: false }
      const id = listedProps(caliper, component)[prop]
      const layered = (component.inheritance ?? []).some((layer: any) => layerMembers(caliper, layer).includes(prop))
      const type = id === undefined ? undefined : caliper.props[id]?.type
      return { found: true, norm: type === undefined ? undefined : caliperNorm(type), layered }
    },
  }
}
