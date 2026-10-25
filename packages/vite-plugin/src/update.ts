import type { IncrementalUpdate } from 'caliper-docgen-napi'

/**
 * What `caliper-docgen:update` carries to the client. Events arrive in `revision` order; each is a delta from the one
 * before. Delete `removedKeys`, set `updatedKeys[i]` to `updatedComponents[i]`, merge `props`, `propSets` and `nameLists`
 * into the output's tables, drop `removedDiagnostics` and add `diagnostics`, and the client holds the same output the virtual module
 * serves, less any props no component refers to any more.
 */
export interface CaliperDocgenUpdate {
  file: string
  revision: number
  updatedKeys: string[]
  updatedComponents: IncrementalUpdate['updatedComponents']
  /** Every prop `updatedComponents` refer to, by id. */
  props: IncrementalUpdate['props']
  /** The prop sets and omitted-names lists `updatedComponents` refer to, by id. */
  propSets: IncrementalUpdate['propSets']
  nameLists: IncrementalUpdate['nameLists']
  removedKeys: string[]
  /** Diagnostics this revision added. */
  diagnostics: IncrementalUpdate['diagnostics']
  /** Diagnostics this revision no longer has. */
  removedDiagnostics: IncrementalUpdate['removedDiagnostics']
}

export function updatePayload(file: string, update: IncrementalUpdate): CaliperDocgenUpdate {
  return {
    file,
    revision: update.revision,
    updatedKeys: update.updatedKeys,
    updatedComponents: update.updatedComponents,
    props: update.props,
    propSets: update.propSets,
    nameLists: update.nameLists,
    removedKeys: update.removedKeys,
    diagnostics: update.diagnostics,
    removedDiagnostics: update.removedDiagnostics,
  }
}
