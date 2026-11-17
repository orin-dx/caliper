// How the oracle pairs a component of the checker with one of Caliper's (ADR 0027): by file and by the name the module
// exports it under, which the canonical JSON gives as `exports`. A display name stands for neither the declaration nor
// the export (`_Card` and `Card` can be two components of one file, `Menu.Item` and `Item` too), so nothing here reads one.

/** One way a module exports a component, as the canonical JSON writes it: the export name (`default` for the default
 * export), then the properties of that export a sub-component attached to it is reached through. */
export interface ComponentExport {
  name: string
  members: string[]
}

/** A component of Caliper's canonical JSON. */
export interface CaliperComponent {
  exports: ComponentExport[]
}

/** The export as a path: `['default', 'Item']` for `Menu.Item = Item` beside `export default Menu`. */
export const pathOfExport = (e: ComponentExport): string[] => [e.name, ...e.members]

/** Identifies an export path within a file, for a map. */
export const exportPathKey = (file: string, path: readonly string[]): string => `${file}\u0000${path.join('\u0000')}`

/** The components of one file that the module itself exports under one of `names`. A sub-component attached to an
 * exported component is reached through that export, not exported as it, so `default.Item` is not an export `default`. */
export function exportedAs<T extends { component: CaliperComponent }>(
  candidates: readonly T[],
  names: readonly string[],
): T[] {
  return candidates.filter(({ component }) =>
    component.exports.some((e) => e.members.length === 0 && names.includes(e.name)),
  )
}
