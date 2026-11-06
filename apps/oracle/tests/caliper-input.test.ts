import { describe, expect, it } from 'vitest'
import { caliperNorm, compare, prim } from '../src/oracle-compare.ts'
import { checkCaliperShape, flattenProps, requireComparison } from '../src/caliper-input.ts'

const good = { schemaVersion: 4, components: {}, propSets: {}, props: {} }

describe('the oracle input checks', () => {
  it('a renamed propSets key stops the run naming the key', () => {
    expect.hasAssertions()
    const { propSets, ...renamed } = { ...good, prop_sets: {} }
    void propSets
    expect(() => checkCaliperShape(renamed)).toThrow('caliper output has no "propSets" object')
  })

  it('a schema version the oracle does not read stops the run', () => {
    expect.hasAssertions()
    expect(() => checkCaliperShape({ ...good, schemaVersion: 3 })).toThrow(
      'caliper output has schemaVersion 3, the oracle reads 4',
    )
    expect(() => checkCaliperShape({ components: {} })).toThrow('has schemaVersion undefined')
  })

  it('a prop set id that propSets lacks names the component and the id', () => {
    expect.hasAssertions()
    expect(() => flattenProps({ propSets: { a: { x: 'p1' } } }, 'Button', { props: ['a', 'gone'] })).toThrow(
      'component Button lists prop set gone, which propSets lacks',
    )
    expect(flattenProps({ propSets: { a: { x: 'p1' }, b: { y: 'p2' } } }, 'Button', { props: ['a', 'b'] })).toEqual({
      x: 'p1',
      y: 'p2',
    })
  })

  it('a comparison of zero shared props throws instead of reading clean', () => {
    expect.hasAssertions()
    expect(() => requireComparison(0, 12)).toThrow('the comparison is empty: 12 components matched, 0 shared props')
    expect(() => requireComparison(3, 1)).not.toThrow()
  })

  it('optional and rest parameters Caliper records are not scored less precise', () => {
    expect.hasAssertions()
    const fn = (optional: boolean, rest: boolean) =>
      caliperNorm({
        kind: 'function',
        params: [{ name: 'a', type: { kind: 'string' }, optional, rest }],
        returnType: { kind: 'void' },
      })
    const checker = {
      k: 'function' as const,
      params: [{ optional: true, rest: false, type: prim('string') }],
      returns: prim('void'),
    }
    expect(compare(fn(true, false), checker)).toBe('identical')
    expect(compare(fn(false, false), checker)).toBe('less precise')
  })
})
