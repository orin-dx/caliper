// The tool adapters are pure, so their Norms are tested without a checker. The inputs are what the tools reported for
// antd's `Segmented` (scripts/rdt.mjs, scripts/rdg.mjs).
import { describe, expect, it } from 'vitest'
import { compare, prim, type Norm } from '../src/oracle-compare.ts'
import { rdgNorm, rdtNorm, typeFromText } from '../src/tool-norm.ts'
import { lit } from './lit.ts'

const union = (...members: Norm[]): Norm => ({ k: 'union', members })
const name = (value: string): Norm => ({ k: 'name', name: value })
const opaque: Norm = { k: 'opaque' }

describe('typeFromText', () => {
  it('reads primitives, literals of either quote, and booleans', () => {
    expect.hasAssertions()
    expect(typeFromText('string')).toStrictEqual(prim('string'))
    expect(typeFromText('"default" | \'round\'')).toStrictEqual(union(lit('default'), lit('round')))
    expect(typeFromText('1.0 | -2 | true | null')).toStrictEqual(union(lit('1'), lit('-2'), lit('true'), prim('null')))
  })

  it('reads a printed function with its parameter markers', () => {
    expect.hasAssertions()
    expect(typeFromText('((value: ValueType) => void)')).toStrictEqual({
      k: 'function',
      params: [{ optional: false, rest: false, type: name('ValueType') }],
      returns: prim('void'),
    })
    expect(typeFromText('(a?: string, ...rest: number[]) => void')).toStrictEqual({
      k: 'function',
      params: [
        { optional: true, rest: false, type: prim('string') },
        { optional: false, rest: true, type: { k: 'array', element: prim('number') } },
      ],
      returns: prim('void'),
    })
  })

  it('reads arrays, tuples and object literals', () => {
    expect.hasAssertions()
    expect(typeFromText('(T | SegmentedLabeledOption<T>)[]')).toStrictEqual({
      k: 'array',
      element: union(name('T'), name('SegmentedLabeledOption')),
    })
    expect(typeFromText('Array<string>')).toStrictEqual({ k: 'array', element: prim('string') })
    expect(typeFromText('readonly [string, number?]')).toStrictEqual({
      k: 'tuple',
      elements: [prim('string'), prim('number')],
    })
    expect(typeFromText('{ root?: string; label: string; [key: string]: unknown }')).toStrictEqual({
      k: 'object',
      fields: ['label', 'root'],
      index: true,
      indexValue: prim('unknown'),
    })
  })

  it('keeps a qualified name as its last segment and drops its type arguments', () => {
    expect.hasAssertions()
    expect(typeFromText('React.ReactNode')).toStrictEqual(name('ReactNode'))
    expect(typeFromText('SegmentedOptions<ValueType>')).toStrictEqual(name('SegmentedOptions'))
    expect(typeFromText('string & {}')).toStrictEqual(prim('string'))
  })

  it('leaves what a tool did not evaluate, or printed in part, opaque', () => {
    expect.hasAssertions()
    expect(typeFromText("SegmentedSemanticAllType['classNamesAndFn']")).toStrictEqual(opaque)
    expect(typeFromText('keyof Foo')).toStrictEqual(opaque)
    expect(typeFromText('DeepClassNameType<...>')).toStrictEqual(opaque)
    // RDT prints a type it could not name as `{string}`, a literal whose member has no type.
    expect(typeFromText('{string}')).toStrictEqual(opaque)
    expect(typeFromText('{ a: string }')).toStrictEqual({ k: 'object', fields: ['a'], index: false })
    expect(typeFromText('')).toStrictEqual(opaque)
  })
})

describe('rdtNorm', () => {
  it('reads a literal union RDT expanded from an alias as the union of its values', () => {
    expect.hasAssertions()
    // `size?: SizeType`, which RDT reports as an enum named by the alias.
    const size = {
      name: 'enum',
      raw: 'SizeType',
      value: [{ value: '"small"' }, { value: '"medium"' }, { value: '"middle"' }, { value: '"large"' }],
    }
    expect(rdtNorm(size)).toStrictEqual({
      k: 'union',
      members: [lit('small'), lit('medium'), lit('middle'), lit('large')],
      alias: 'SizeType',
    })
  })

  it('reads any other type from its printed name', () => {
    expect.hasAssertions()
    expect(rdtNorm({ name: 'boolean' })).toStrictEqual(prim('boolean'))
    expect(rdtNorm({ name: '((value: ValueType) => void)' })).toMatchObject({ k: 'function' })
  })

  it('has no type for an enum without values or a missing type object', () => {
    expect.hasAssertions()
    expect(rdtNorm({ name: 'enum' })).toStrictEqual(opaque)
    expect(rdtNorm(undefined)).toStrictEqual(opaque)
  })

  it('is judged by the same rules as Caliper: a literal union against the checker, and a lost optional marker', () => {
    expect.hasAssertions()
    const checker = union(lit('small'), lit('large'), prim('undefined'))
    expect(compare(rdtNorm({ name: 'enum', value: [{ value: '"small"' }, { value: '"large"' }] }), checker)).toBe(
      'identical',
    )
    // `string` where the checker has a literal union is looser, whichever tool says it.
    expect(compare(rdtNorm({ name: 'string' }), checker)).toBe('less precise')
    // A tool that keeps the optional marker is not capped by it.
    const withMarker: Norm = {
      k: 'function',
      params: [{ optional: true, rest: false, type: prim('string') }],
      returns: prim('void'),
    }
    expect(compare(rdtNorm({ name: '(value?: string) => void' }), withMarker)).toBe('identical')
  })
})

describe('rdgNorm', () => {
  it('reads the source text react-docgen kept', () => {
    expect.hasAssertions()
    // `size?: 'small' | 'medium' | 'middle' | 'large'`, with its `undefined` member.
    const size = {
      name: 'union',
      raw: "'small' | 'medium' | 'middle' | 'large' | undefined",
      elements: [{ name: 'literal', value: "'small'" }],
    }
    expect(rdgNorm(size)).toStrictEqual(
      union(lit('small'), lit('medium'), lit('middle'), lit('large'), prim('undefined')),
    )
  })

  it('leaves a type it could not resolve as the text it printed', () => {
    expect.hasAssertions()
    // `classNames?: SegmentedSemanticAllType['classNamesAndFn']`: a union by name, with no elements to read.
    expect(rdgNorm({ name: 'union', raw: "SegmentedSemanticAllType['classNamesAndFn']" })).toStrictEqual(opaque)
    // An array of an unresolved type parameter: its elements say `unknown`, its raw text says what was written.
    expect(
      rdgNorm({ name: 'Array', raw: '(T | SegmentedLabeledOption<T>)[]', elements: [{ name: 'unknown' }] }),
    ).toStrictEqual({ k: 'array', element: union(name('T'), name('SegmentedLabeledOption')) })
  })

  it('builds from elements and names where there is no raw text', () => {
    expect.hasAssertions()
    expect(rdgNorm({ name: 'string' })).toStrictEqual(prim('string'))
    expect(rdgNorm({ name: 'ReactNode' })).toStrictEqual(name('ReactNode'))
    expect(rdgNorm({ name: 'literal', value: "'a'" })).toStrictEqual(lit('a'))
    expect(rdgNorm({ name: 'union', elements: [{ name: 'string' }, { name: 'number' }] })).toStrictEqual(
      union(prim('string'), prim('number')),
    )
    expect(rdgNorm({ name: 'Array', elements: [{ name: 'string' }] })).toStrictEqual({
      k: 'array',
      element: prim('string'),
    })
  })

  it('does not read a kind of type as a type of that name', () => {
    expect.hasAssertions()
    expect(rdgNorm({ name: 'signature' })).toStrictEqual(opaque)
    expect(rdgNorm({ name: 'union' })).toStrictEqual(opaque)
    expect(rdgNorm(undefined)).toStrictEqual(opaque)
  })
})

describe('index signatures and literal kinds in tool text', () => {
  it('keeps an index signature value type', () => {
    expect.hasAssertions()
    expect(typeFromText('{ [key: string]: string }')).toStrictEqual({
      k: 'object',
      fields: [],
      index: true,
      indexValue: prim('string'),
    })
    expect(typeFromText('{ a: number; [key: string]: string | number }')).toMatchObject({
      fields: ['a'],
      indexValue: union(prim('string'), prim('number')),
    })
  })

  it('a reported index signature of another value type is wrong, not identical', () => {
    expect.hasAssertions()
    const checker: Norm = { k: 'object', fields: [], index: true, indexValue: prim('number') }
    expect(compare(typeFromText('{ [key: string]: string }'), checker)).toBe('wrong')
    expect(compare(typeFromText('{ [key: string]: number }'), checker)).toBe('identical')
  })

  it('reads a quoted literal as a string and a bare one as a number', () => {
    expect.hasAssertions()
    expect(typeFromText('"0"')).toStrictEqual(lit('0', 'string'))
    expect(typeFromText('0')).toStrictEqual(lit('0', 'number'))
    expect(typeFromText('-1')).toStrictEqual(lit('-1', 'number'))
    expect(typeFromText('true')).toStrictEqual(lit('true', 'boolean'))
    expect(typeFromText('"true"')).toStrictEqual(lit('true', 'string'))
  })
})
