// Which files a component's collapsed inheritance layers cover.
import { describe, expect, it } from 'vitest'
import { collapsedLayerFiles, pathKey } from '../src/tool-universe.ts'

const BIG = '/r/node_modules/dep/big.d.ts'
const OTHER = '/r/node_modules/dep/other.d.ts'

describe('collapsedLayerFiles', () => {
  it('keeps a layer collapsed once Omit leaves it 30 props, because it still lists members', () => {
    const members = Array.from({ length: 30 }, (_, i) => `p${i + 1}`)
    const component = { inheritance: [{ typeName: 'Big', fileName: BIG, totalProps: 30, members }] }
    expect([...collapsedLayerFiles({}, component)]).toStrictEqual([pathKey(BIG)])
  })

  it('reads members interned in nameLists', () => {
    const caliper = { nameLists: { n1: ['a', 'b'] } }
    const component = { inheritance: [{ typeName: 'Small', fileName: OTHER, totalProps: 2, members: 'n1' }] }
    expect([...collapsedLayerFiles(caliper, component)]).toStrictEqual([pathKey(OTHER)])
  })

  it('does not count an expanded layer, which lists no members', () => {
    const component = { inheritance: [{ typeName: 'Small', fileName: OTHER, totalProps: 12 }] }
    expect(collapsedLayerFiles({}, component).size).toBe(0)
  })

  it('still reads an output with no members by its size', () => {
    const component = { inheritance: [{ typeName: 'Big', fileName: BIG, totalProps: 31 }] }
    expect([...collapsedLayerFiles({}, component)]).toStrictEqual([pathKey(BIG)])
  })
})
