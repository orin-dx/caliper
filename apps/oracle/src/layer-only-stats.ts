// What the layer-only props of one corpus are (src/layer-only.ts), how the collapse threshold sorts them, and what a rule
// by relevance would list that the size rule does not. Pure apart from the `read` it is given.
import { classify, layerOnly, topApi, type Kind, type LayerOnly, type TopApi, type Why } from './layer-only.ts'
import { layerMembers, listedProps } from './canonical.ts'

/** The collapse threshold: a dependency source declaring more props than this is one layer, not a list of props. */
export const THRESHOLD = 30

export interface Input {
  /** The canonical JSON at the default collapse (`dependencies`). */
  caliper: any
  /** The same run with `--collapse-inherited-props none`, for what listing the props would add. */
  expanded?: any
  /** The two JSON documents' sizes in bytes. */
  bytes?: { default: number; expanded: number }
}

const count = (values: string[]) => {
  const out: Record<string, number> = {}
  for (const v of values) out[v] = (out[v] ?? 0) + 1
  return out
}

const bucket = (size: number) => {
  if (size <= THRESHOLD) return `<=${THRESHOLD}`
  if (size <= 40) return '31-40'
  return size <= 60 ? '41-60' : '61+'
}

/** A layer's class by its file and type alone, which is what clauses 1 to 3 read. */
const layerKind = (l: { layer: string; file: string }): Kind =>
  classify({ layer: l.layer, layerFile: l.file, prop: '' }).kind

/** The dependency sources whose props the output lists one by one: each prop records the type that declares it (`parent`),
 * so a source is a distinct type and file, and its size the number of its props listed. Not `@types/react`'s. A collapsed
 * source is not one: it still lists its curated props (`className`) with itself as their parent. */
export function expandedDependencySources(caliper: any): { layer: string; file: string; size: number }[] {
  const collapsed = new Set<string>()
  for (const component of Object.values<any>(caliper.components)) {
    for (const layer of component.inheritance ?? []) {
      if (layerMembers(caliper, layer).length > 0) collapsed.add(`${layer.typeName}\u0000${layer.fileName}`)
    }
  }
  const sources = new Map<string, { layer: string; file: string; names: Set<string> }>()
  for (const prop of Object.values<any>(caliper.props)) {
    const { parent } = prop
    const file = parent ? String(parent.fileName) : ''
    if (!file.includes('/node_modules/') || file.includes('/@types/react/')) continue
    const key = `${parent.name}\u0000${file}`
    if (collapsed.has(key)) continue
    const source = sources.get(key) ?? { layer: parent.name, file, names: new Set() }
    source.names.add(prop.name)
    sources.set(key, source)
  }
  return [...sources.values()].map((s) => ({ layer: s.layer, file: s.file, size: s.names.size }))
}

/** The layers of a corpus, once each by type and file: collapsed (they list `members`), expanded dependency sources,
 * and the HTML-element layers React's attributes make. */
export function layerCounts(caliper: any) {
  const layers = new Map<string, { layer: string; file: string; size: number; collapsed: boolean; html: boolean }>()
  for (const component of Object.values<any>(caliper.components)) {
    for (const layer of component.inheritance ?? []) {
      const key = `${layer.typeName}\u0000${layer.fileName}`
      const collapsed = layerMembers(caliper, layer).length > 0
      layers.set(key, {
        layer: layer.typeName,
        file: String(layer.fileName),
        size: layer.totalProps,
        collapsed,
        html: Boolean(layer.htmlElement),
      })
    }
  }
  const all = [...layers.values()]
  const collapsed = all.filter((l) => l.collapsed)
  const expanded = expandedDependencySources(caliper)
  return {
    distinct_layers: all.length,
    html_layers: all.filter((l) => l.html).length,
    collapsed: {
      layers: collapsed.length,
      above_threshold: collapsed.filter((l) => l.size > THRESHOLD).length,
      at_or_below_threshold: collapsed.filter((l) => l.size <= THRESHOLD).length,
      by_size: count(collapsed.map((l) => bucket(l.size))),
      ambient_by_type_or_file: collapsed.filter((l) => layerKind(l) === 'ambient').length,
    },
    expanded_dependency: {
      layers: expanded.length,
      above_threshold: expanded.filter((l) => l.size > THRESHOLD).length,
      at_or_below_threshold: expanded.filter((l) => l.size <= THRESHOLD).length,
      ambient_by_type_or_file: expanded.filter((l) => layerKind(l) === 'ambient').length,
    },
  }
}

/** How many component props a document lists: each component's own and curated inherited ones. */
const listedCount = (caliper: any) =>
  Object.values<any>(caliper.components).reduce((n, c) => n + Object.keys(listedProps(caliper, c)).length, 0)

/** The configuration with nothing collapsed (`--collapse-inherited-props none`), which is the Storybook addon's, next to
 * the default one (docs/measurement.md, "Which configuration this describes"). Nothing is collapsed there. */
function expandedConfig(caliper: any, expanded: any) {
  return {
    collapsed_layers: layerCounts(expanded).collapsed.layers,
    layer_only_props: layerOnly(expanded).length,
    listed_props: { default: listedCount(caliper), expanded: listedCount(expanded) },
  }
}

/** What listing a set of layer-only props would add to the canonical JSON, from the fully expanded run: how many of
 * them it lists, and the bytes of their distinct prop entries plus one `name: id` per component. */
function listing(items: LayerOnly[], expanded: any) {
  const ids = new Set<string>()
  let listed = 0
  let nameBytes = 0
  for (const item of items) {
    const component = expanded.components[item.key]
    const id = component ? listedProps(expanded, component)[item.prop] : undefined
    if (id === undefined) continue
    listed++
    ids.add(id)
    nameBytes += item.prop.length + id.length + 6
  }
  const entryBytes = [...ids].reduce((sum, id) => sum + JSON.stringify(expanded.props[id]).length, 0)
  return { props: listed, distinct_entries: ids.size, bytes_estimate: entryBytes + nameBytes }
}

/** The ambient props a dependency source lists one by one today (the source is small, so the size rule leaves it
 * expanded): how many component props, and the bytes of their distinct entries plus one `name: id` per component. A
 * relevance rule would collapse them. */
export function ambientListedToday(caliper: any) {
  const ids = new Set<string>()
  let props = 0
  let nameBytes = 0
  for (const component of Object.values<any>(caliper.components)) {
    for (const [name, id] of Object.entries(listedProps(caliper, component))) {
      const parent = caliper.props[id]?.parent
      const file = parent ? String(parent.fileName) : ''
      if (!file.includes('/node_modules/') || file.includes('/@types/react/')) continue
      if (classify({ layer: parent.name, layerFile: file, prop: name }).kind !== 'ambient') continue
      props++
      ids.add(id)
      nameBytes += name.length + id.length + 6
    }
  }
  const entryBytes = [...ids].reduce((sum, id) => sum + JSON.stringify(caliper.props[id]).length, 0)
  return { props, distinct_entries: ids.size, bytes_estimate: entryBytes + nameBytes }
}

export function analyse(input: Input, read: (file: string) => string | undefined, limit = 20) {
  const items = layerOnly(input.caliper)
  const classified = items.map((item) => ({ item, ...classify(item) }))
  const of = (kind: Kind) => classified.filter((c) => c.kind === kind)
  const distinct = (kind: Kind) =>
    new Set(of(kind).map((c) => `${c.item.layer}\u0000${c.item.layerFile}\u0000${c.item.prop}`)).size
  // A collapsed layer is judged by the props of it that no component lists: API if any of those is API.
  const byLayer = new Map<string, Set<Kind>>()
  for (const c of classified) {
    const key = `${c.item.layer}\u0000${c.item.layerFile}`
    byLayer.set(key, (byLayer.get(key) ?? new Set()).add(c.kind))
  }
  const api = of('api').map((c) => c.item)
  const near = api.filter((i) => i.layerSize > THRESHOLD && i.layerSize <= 40)
  const effect = input.expanded
    ? {
        api: listing(api, input.expanded),
        ambient: listing(
          of('ambient').map((c) => c.item),
          input.expanded,
        ),
        all_expanded_bytes_delta: input.bytes ? input.bytes.expanded - input.bytes.default : undefined,
        default_bytes: input.bytes?.default,
      }
    : undefined
  const top: TopApi[] = topApi(items, limit, read)
  return {
    components: Object.keys(input.caliper.components).length,
    layer_only: {
      props: items.length,
      api: of('api').length,
      ambient: of('ambient').length,
      by_clause: count(classified.map((c): Why => c.why)),
      distinct_api: distinct('api'),
      distinct_ambient: distinct('ambient'),
      components_with_api: new Set(api.map((i) => i.key)).size,
    },
    layers: layerCounts(input.caliper),
    size_rule_against_relevance: {
      collapsed_layers_with_api_props: [...byLayer.values()].filter((kinds) => kinds.has('api')).length,
      collapsed_layers_all_ambient: [...byLayer.values()].filter((kinds) => !kinds.has('api')).length,
      api_props_in_layers_of_31_to_40: near.length,
    },
    effect,
    expanded_config: input.expanded ? expandedConfig(input.caliper, input.expanded) : undefined,
    ambient_listed_today: ambientListedToday(input.caliper),
    top_api: top,
  }
}
