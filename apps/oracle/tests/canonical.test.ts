// The canonical JSON's shared shapes, read the way every oracle script reads them.
import { describe, expect, it } from 'vitest'
import { layerMembers, listedProps, propMap } from '../src/canonical.ts'

const caliper = {
  propSets: { a: { x: 'px', y: 'py' }, b: { z: 'pz' } },
  nameLists: { n: ['onCopy', 'onCut'] },
}

describe('propMap', () => {
  it('reads a map as it is, and an array of prop-set ids as one merged map', () => {
    expect.hasAssertions()
    expect(propMap(caliper, { x: 'px' })).toStrictEqual({ x: 'px' })
    expect(propMap(caliper, ['a', 'b'])).toStrictEqual({ x: 'px', y: 'py', z: 'pz' })
  })

  it('reads nothing as an empty map', () => {
    expect.hasAssertions()
    expect(propMap(caliper, undefined)).toStrictEqual({})
  })
})

describe('listedProps', () => {
  it('lists the curated inherited props after the component’s own, and an own prop wins a clash', () => {
    expect.hasAssertions()
    const component = { props: { x: 'own-x' }, notableInherited: { x: 'notable-x', w: 'notable-w' } }
    expect(listedProps(caliper, component)).toStrictEqual({ x: 'own-x', w: 'notable-w' })
  })

  it('reads schema 2’s arrays in both lists, and a component with no curated props', () => {
    expect.hasAssertions()
    expect(listedProps(caliper, { props: ['a'], notableInherited: ['b'] })).toStrictEqual({ x: 'px', y: 'py', z: 'pz' })
    expect(listedProps(caliper, { props: { x: 'px' } })).toStrictEqual({ x: 'px' })
  })
})

describe('layerMembers', () => {
  it('resolves an interned name list, and reads an inline one as it is', () => {
    expect.hasAssertions()
    expect(layerMembers(caliper, { members: 'n' })).toStrictEqual(['onCopy', 'onCut'])
    expect(layerMembers(caliper, { members: ['a'] })).toStrictEqual(['a'])
  })

  it('has no members for a layer without any, or one whose list is not in the document', () => {
    expect.hasAssertions()
    expect(layerMembers(caliper, {})).toStrictEqual([])
    expect(layerMembers(caliper, { members: 'missing' })).toStrictEqual([])
  })
})
