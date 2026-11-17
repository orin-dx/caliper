// The canonical JSON's shared shapes, read the way the oracle reads them.
import { describe, expect, it } from 'vitest'
import { layerMembers } from '../src/canonical.ts'

const caliper = {
  nameLists: { n: ['onCopy', 'onCut'] },
}

describe('layerMembers', () => {
  it('reads an interned name list, and an inline one', () => {
    expect.hasAssertions()
    expect(layerMembers(caliper, { members: 'n' })).toStrictEqual(['onCopy', 'onCut'])
    expect(layerMembers(caliper, { members: ['a'] })).toStrictEqual(['a'])
  })

  it('reads a missing or unknown list as empty', () => {
    expect.hasAssertions()
    expect(layerMembers(caliper, {})).toStrictEqual([])
    expect(layerMembers(caliper, { members: 'missing' })).toStrictEqual([])
  })
})
