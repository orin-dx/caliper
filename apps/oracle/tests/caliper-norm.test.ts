// `caliperNorm` reads every kind of the published `PropType` (packages/napi/index.d.ts): a kind added there fails the
// type check of SAMPLES below and of the switch in src/caliper-norm.ts, and a kind the build does not know throws.
import { describe, expect, it } from 'vitest'
import type { PropType } from '../../../packages/napi/index.d.ts'
import { caliperNorm } from '../src/caliper-norm.ts'
import { prim, type Norm } from '../src/oracle-compare.ts'
import { lit } from './lit.ts'

type Kind = PropType['kind']
type Sample<K extends Kind> = Extract<PropType, { kind: K }>

const string: PropType = { kind: 'string' }

/** One value of each kind, and the `Norm` it reads as. */
const SAMPLES: { [K in Kind]: [Sample<K>, Norm] } = {
  string: [{ kind: 'string' }, prim('string')],
  number: [{ kind: 'number' }, prim('number')],
  boolean: [{ kind: 'boolean' }, prim('boolean')],
  null: [{ kind: 'null' }, prim('null')],
  undefined: [{ kind: 'undefined' }, prim('undefined')],
  any: [{ kind: 'any' }, prim('any')],
  never: [{ kind: 'never' }, prim('never')],
  unknown: [{ kind: 'unknown' }, prim('unknown')],
  void: [{ kind: 'void' }, prim('void')],
  bigint: [{ kind: 'bigint' }, prim('bigint')],
  reactNode: [{ kind: 'reactNode' }, { k: 'name', name: 'ReactNode' }],
  cssProperties: [{ kind: 'cssProperties' }, { k: 'name', name: 'CSSProperties' }],
  elementType: [{ kind: 'elementType' }, { k: 'name', name: 'ElementType' }],
  sxProps: [{ kind: 'sxProps' }, { k: 'name', name: 'SxProps' }],
  stringLiteral: [{ kind: 'stringLiteral', value: 'a' }, lit('a', 'string')],
  numberLiteral: [{ kind: 'numberLiteral', value: 1 }, lit('1', 'number')],
  boolLiteral: [{ kind: 'boolLiteral', value: true }, lit('true', 'boolean')],
  bigintLiteral: [{ kind: 'bigintLiteral', value: '-10' }, lit('-10', 'bigint')],
  uniqueSymbol: [{ kind: 'uniqueSymbol', name: 'brand' }, prim('unique symbol')],
  enumMember: [
    { kind: 'enumMember', enum: 'Size', member: 'Small', value: { kind: 'stringLiteral', value: 'small' } },
    lit('small', 'string'),
  ],
  templateLiteral: [{ kind: 'templateLiteral', parts: [{ type: { kind: 'number' } }, { text: 'px' }] }, prim('string')],
  union: [
    { kind: 'union', members: [string, { kind: 'number' }] },
    { k: 'union', members: [prim('string'), prim('number')] },
  ],
  intersection: [
    { kind: 'intersection', members: [string, { kind: 'number' }] },
    { k: 'intersection', members: [prim('string'), prim('number')] },
  ],
  array: [
    { kind: 'array', element: string },
    { k: 'array', element: prim('string') },
  ],
  tuple: [
    { kind: 'tuple', elements: [{ type: string, label: 'head' }] },
    { k: 'tuple', elements: [prim('string')] },
  ],
  readonly: [
    { kind: 'readonly', element: { kind: 'array', element: string } },
    { k: 'array', element: prim('string') },
  ],
  object: [
    {
      kind: 'object',
      fields: [{ name: 'b', propType: string, required: true, description: '' }],
    },
    { k: 'object', fields: ['b'], index: false },
  ],
  callable: [
    {
      kind: 'callable',
      signatures: [{ typeParams: [], params: [], returnType: { kind: 'void' } }],
      fields: [],
    },
    { k: 'function', params: [], returns: prim('void') },
  ],
  named: [
    { kind: 'named', name: 'React.Foo', args: [] },
    { k: 'name', name: 'Foo' },
  ],
  recursive: [
    { kind: 'recursive', name: 'Tree', args: [] },
    { k: 'name', name: 'Tree' },
  ],
  truncated: [{ kind: 'truncated', text: 'A', limit: { type: 'depth' } }, { k: 'opaque' }],
  function: [
    { kind: 'function', typeParams: [], params: [{ name: 'a', type: string }], returnType: { kind: 'void' } },
    { k: 'function', params: [{ optional: false, rest: false, type: prim('string') }], returns: prim('void') },
  ],
  ref: [
    { kind: 'ref', name: 'RefObject', element: null },
    { k: 'name', name: 'Ref' },
  ],
  htmlAttributes: [
    { kind: 'htmlAttributes', element: 'div', omitted: ['color'] },
    { k: 'name', name: 'HTMLAttributes', family: { element: 'div', omitted: ['color'] } },
  ],
  literalUnion: [
    { kind: 'literalUnion', members: ['a', 'b'], hasDefault: false },
    { k: 'union', members: [lit('a', 'string'), lit('b', 'string')] },
  ],
  opaque: [{ kind: 'opaque', raw: 'T extends U ? X : Y', reason: { type: 'conditionalType' } }, { k: 'opaque' }],
}

describe('caliperNorm', () => {
  it.each(Object.entries(SAMPLES))('reads a %s', (kind, [type, expected]) => {
    expect(type.kind).toBe(kind)
    expect(caliperNorm(type)).toStrictEqual(expected)
  })

  it('a kind this build has no case for throws instead of comparing as opaque', () => {
    expect.hasAssertions()
    // @ts-expect-error a kind that is not in the published `PropType`
    const added: PropType = { kind: 'brandNewKind' }

    expect(() => caliperNorm(added)).toThrow('caliperNorm has no case for the PropType {"kind":"brandNewKind"}')
    expect(() => caliperNorm({ kind: 'array', element: added })).toThrow('brandNewKind')
  })

  it('a type nested deeper than the limit is opaque, not an error', () => {
    expect.hasAssertions()
    let nested: PropType = string
    for (let i = 0; i < 6; i++) nested = { kind: 'array', element: nested }

    expect(caliperNorm(nested)).toStrictEqual({
      k: 'array',
      element: { k: 'array', element: { k: 'array', element: { k: 'array', element: { k: 'opaque' } } } },
    })
  })
})
