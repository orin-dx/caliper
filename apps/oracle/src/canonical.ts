// Caliper's canonical `extract --json` as the oracle reads it. The output interns what components share, so a collapsed
// layer's member names need one step to read. Pure.

/** The prop names a collapsed layer lists. The canonical JSON interns the list: `members` is an id into `nameLists`; an
 * older output lists the names inline. */
export function layerMembers(caliper: any, layer: any): string[] {
  const { members } = layer
  if (typeof members === 'string') return caliper.nameLists?.[members] ?? []
  return members ?? []
}
