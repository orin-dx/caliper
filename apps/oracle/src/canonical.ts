// Caliper's canonical `extract --json` as the oracles read it. The output interns what components share, so a component's
// props, its curated inherited props and a collapsed layer's member names each need one step to read; they are here so
// every script reads them the same way. Pure.

/** A map of prop name to prop id as one record. A component's `props` and `notableInherited` are that map inline, or (schema
 * 2) an array of prop-set ids, each such a map in `propSets`. */
export function propMap(caliper: any, list: unknown): Record<string, string> {
  if (Array.isArray(list)) return Object.assign({}, ...list.map((id: string) => caliper.propSets[id]))
  return (list as Record<string, string> | undefined) ?? {}
}

/** The props a props table lists for a component: its own, then the curated props inherited from a collapsed source
 * (`notableInherited`, `crates/core/src/pipeline/rdt.rs`). A prop named in both is the component's own. */
export function listedProps(caliper: any, component: any): Record<string, string> {
  return { ...propMap(caliper, component.notableInherited), ...propMap(caliper, component.props) }
}

/** The prop names a collapsed layer lists. The canonical JSON interns the list: `members` is an id into `nameLists`; an
 * older output lists the names inline. */
export function layerMembers(caliper: any, layer: any): string[] {
  const { members } = layer
  if (typeof members === 'string') return caliper.nameLists?.[members] ?? []
  return members ?? []
}
