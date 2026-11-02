// The oracle's comparison is pure, so its verdicts are tested without a checker.
import { describe, expect, it } from 'vitest'
import { compare, prim, type Norm } from '../src/oracle-compare.ts'

const lit = (value: string): Norm => ({ k: 'lit', value })
const union = (...members: Norm[]): Norm => ({ k: 'union', members })
const undef = prim('undefined')
const optional = (type: Norm): Norm => union(type, undef)

describe('the oracle comparison', () => {
  it('a type parameter Caliper replaced with its default is one instance of it, not another type', () => {
    expect.hasAssertions()
    // `multiple?: Multiple` with `Multiple extends boolean = false`, which Caliper reads as `false`.
    const multiple: Norm = { k: 'param', name: 'Multiple', fallback: lit('false'), constraint: prim('boolean') }
    expect(compare(lit('false'), optional(multiple))).toBe('less precise')
    expect(compare(prim('boolean'), optional(multiple))).toBe('less precise')
  })

  it('a parameter with neither a default nor a constraint holds whatever instance Caliper picked', () => {
    expect.hasAssertions()
    // antd's `Segmented`: `defaultValue?: ValueType` where the cast over `forwardRef` declares `<ValueType>` bare and
    // the props interface declares the default Caliper used.
    const valueType: Norm = { k: 'param', name: 'ValueType' }
    expect(compare(union(prim('string'), prim('number')), optional(valueType))).toBe('less precise')
  })

  it('a literal a primitive of its kind in the same union covers adds nothing', () => {
    expect.hasAssertions()
    // antd's `DrawerPanel`: `size?: 'default' | 'large' | number | string`, which the checker reduces to `string | number`.
    const written = union(lit('default'), lit('large'), prim('number'), prim('string'))
    expect(compare(written, optional(union(prim('string'), prim('number'))))).toBe('identical')
    // Without the primitive the literals are still the type.
    expect(compare(union(lit('default'), lit('large')), optional(union(lit('default'), lit('large'))))).toBe(
      'identical',
    )
    // A numeric literal is covered by `number`, not by `string`.
    expect(compare(union(lit('1'), prim('string')), optional(union(lit('1'), prim('string'))))).toBe('identical')
  })

  it('a type parameter replaced with its constraint is one instance of it', () => {
    expect.hasAssertions()
    // `value?: Value` with `Value extends string`, which Caliper reads as `string`.
    const value: Norm = { k: 'param', name: 'Value', constraint: prim('string') }
    expect(compare(prim('string'), optional(value))).toBe('less precise')
  })

  it('a type that is neither the default nor the constraint of the parameter is wrong', () => {
    expect.hasAssertions()
    const value: Norm = { k: 'param', name: 'Value', fallback: prim('string'), constraint: prim('string') }
    expect(compare(prim('number'), optional(value))).toBe('wrong')
  })

  it('a union Caliper wrote out fits a parameter whose default is that union', () => {
    expect.hasAssertions()
    // `value?: ValueType` with `ValueType extends SegmentedValue = SegmentedValue`, and `SegmentedValue = string | number`.
    const both = union(prim('string'), prim('number'))
    const valueType: Norm = { k: 'param', name: 'ValueType', fallback: both, constraint: both }
    expect(compare(both, optional(valueType))).toBe('less precise')
    expect(compare(union(prim('string'), prim('boolean')), optional(valueType))).toBe('wrong')
  })

  it('a type parameter inside an array is compared through the same instances', () => {
    expect.hasAssertions()
    const value: Norm = { k: 'param', name: 'Value', constraint: prim('string') }
    const checker: Norm = { k: 'array', element: value }
    expect(compare({ k: 'array', element: prim('string') }, checker)).toBe('less precise')
    expect(compare({ k: 'array', element: prim('number') }, checker)).toBe('wrong')
  })

  describe('function parameter markers', () => {
    const param = (type: Norm, marks: { optional?: boolean; rest?: boolean } = {}) => ({
      optional: false,
      rest: false,
      ...marks,
      type,
    })
    const fn = (...params: ReturnType<typeof param>[]): Norm => ({ k: 'function', params, returns: prim('void') })

    it('a side that records neither marker is less precise than a checker that has one', () => {
      expect.hasAssertions()
      // Caliper's own functions: `(value?: string) => void` and `(...args: string[]) => void` read without markers.
      expect(compare(fn(param(prim('string'))), fn(param(prim('string'), { optional: true })))).toBe('less precise')
      expect(compare(fn(param(prim('string'))), fn(param(prim('string'), { rest: true })))).toBe('less precise')
    })

    it('a side that keeps the markers is not capped by them', () => {
      expect.hasAssertions()
      const marked = fn(param(prim('string'), { optional: true }))
      const spread = fn(param(prim('string'), { rest: true }))
      expect(compare(marked, marked)).toBe('identical')
      expect(compare(spread, spread)).toBe('identical')
    })

    it('a marker only the other side has does not lower the verdict, and a lost one still does', () => {
      expect.hasAssertions()
      const plain = fn(param(prim('string')))
      const marked = fn(param(prim('string'), { optional: true }))
      expect(compare(marked, plain)).toBe('identical')
      // The optional marker is kept but the rest marker is lost.
      const both = fn(param(prim('string'), { optional: true, rest: true }))
      expect(compare(marked, both)).toBe('less precise')
    })

    it('a parameter the side lacks is less precise when the checker marks it and wrong when it does not', () => {
      expect.hasAssertions()
      const one = fn(param(prim('string')))
      expect(compare(one, fn(param(prim('string')), param(prim('number'), { optional: true })))).toBe('less precise')
      expect(compare(one, fn(param(prim('string')), param(prim('number'))))).toBe('wrong')
    })
  })

  it('an optional prop of type never is undefined in the checker', () => {
    expect.hasAssertions()
    // `classNames?: never` reads `undefined`, the optional marker the comparison drops.
    expect(compare(prim('never'), undef)).toBe('identical')
    expect(compare(prim('never'), prim('string'))).toBe('wrong')
    expect(compare(prim('string'), undef)).toBe('wrong')
  })
})
