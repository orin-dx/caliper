// Which files a component's collapsed inheritance layers cover, and which functions the universe counts.
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { prim } from '../src/oracle-compare.ts'
import { collapsedLayerFiles, pathKey, universeRecorder } from '../src/tool-universe.ts'

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

  it('does not count an expanded layer of more than 30 props, which `none` leaves without members', () => {
    const component = { inheritance: [{ typeName: 'Big', fileName: BIG, totalProps: 45 }] }
    expect(collapsedLayerFiles({}, component).size).toBe(0)
  })
})

describe('universeRecorder', () => {
  let dir: string
  let file: string
  const prop = (name: string) => ({ name, declarations: [{ path: file }] })
  const checker = { getTypeOfSymbol: (symbols: unknown[]) => symbols.map(() => ({})) }
  const counts = { identical: 0, spelled: 0, 'less precise': 0, wrong: 0, 'checker any/error': 0 }

  beforeAll(() => {
    dir = realpathSync(mkdtempSync(join(tmpdir(), 'tool-universe-')))
    file = join(dir, 'Button.tsx')
    writeFileSync(file, '')
  })

  afterAll(() => rmSync(dir, { recursive: true, force: true }))

  const recorder = () =>
    universeRecorder({ root: dir, caliper: { props: {} }, checker, checkerNorm: () => prim('string') })

  it('leaves a function the shape rules exclude out of the score, and keeps it in the one over every function', () => {
    expect.hasAssertions()
    const universe = recorder()
    universe.record({ own: [prop('size'), prop('label')], name: 'Button', file, matches: [] })
    universe.record({ own: [prop('isOpen')], name: 'useMenu', file, matches: [], nonComponent: 'hook' })
    universe.record({
      own: [prop('a'), prop('b'), prop('c')],
      name: 'pick',
      file,
      matches: [],
      nonComponent: 'not_jsx',
    })

    const { caliper, rdt } = universe.score(counts)

    expect(caliper).toMatchObject({ universe: 2, components: 1 })
    expect(rdt).toBeUndefined()
    expect(universe.scoreAll().caliper).toMatchObject({ universe: 6, components: 3 })
  })

  it('counts what it left out, by the rule that left it out', () => {
    expect.hasAssertions()
    const universe = recorder()
    universe.record({ own: [prop('a')], name: 'Button', file, matches: [] })
    universe.record({ own: [prop('isOpen'), prop('x')], name: 'useMenu', file, matches: [], nonComponent: 'hook' })
    universe.record({ own: [prop('y')], name: 'useTree', file, matches: [], nonComponent: 'hook' })
    universe.record({ own: [prop('charAt')], name: 'label', file, matches: [], nonComponent: 'lib_members' })

    expect(universe.excluded()).toStrictEqual({
      components: 3,
      props: 4,
      by_reason: { hook: { components: 2, props: 3 }, lib_members: { components: 1, props: 1 } },
    })
  })

  it('keeps a function Caliper reports as a component, whatever its shape', () => {
    expect.hasAssertions()
    const universe = recorder()
    const match = { name: 'useMenu', key: 'useMenu', component: { props: {}, inheritance: [] } }
    universe.record({ own: [prop('isOpen')], name: 'useMenu', file, matches: [match], nonComponent: 'hook' })

    expect(universe.score(counts).caliper).toMatchObject({ universe: 1 })
    expect(universe.excluded()).toStrictEqual({ components: 0, props: 0, by_reason: {} })
  })
})
