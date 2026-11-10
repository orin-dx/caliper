// The oracle's comparison is pure, so its verdicts are tested without a checker.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { caliperNorm } from '../src/caliper-norm.ts'
import { compare, prim, type Norm } from '../src/oracle-compare.ts'
import { lit } from './lit.ts'

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

  describe('comparator artifacts', () => {
    // Caliper's canonical JSON for `{ [id: string]: V }`: one field, named after the signature's parameter, whose
    // `indexKey` is the key type.
    const indexSignature = (name: string, value: object) => ({
      kind: 'object',
      fields: [{ name, propType: value, required: true, description: '', indexKey: { kind: 'string' } }],
    })
    const checkerIndex = (value: Norm): Norm => ({ k: 'object', fields: [], index: true, indexValue: value })

    it('an index signature is read as an index, not as a property named after its parameter', () => {
      expect.hasAssertions()
      // shadcn's `defaultLayout?: Layout` with `Layout = { [id: string]: number }`.
      const caliper = caliperNorm(indexSignature('id', { kind: 'number' }))
      expect(caliper).toMatchObject({ k: 'object', fields: [], index: true })
      expect(compare(caliper, optional(checkerIndex(prim('number'))))).toBe('identical')
      // The parameter's name is not a property under any spelling.
      expect(compare(caliperNorm(indexSignature('name', { kind: 'string' })), checkerIndex(prim('string')))).toBe(
        'identical',
      )
    })

    it('a member keyed by a symbol is the checker side’s `__@name` member', () => {
      expect.hasAssertions()
      // `{ [tag]: string; name: string }`, which the checker keys `__@tag@<id>` and src/oracle-checker.ts reads `__@tag`.
      const caliper = caliperNorm({
        kind: 'object',
        fields: [
          { name: '[tag]', propType: { kind: 'string' }, required: true, description: '' },
          { name: '[Symbol.iterator]', propType: { kind: 'string' }, required: true, description: '' },
          { name: 'name', propType: { kind: 'string' }, required: true, description: '' },
        ],
      })
      expect(caliper).toMatchObject({ fields: ['__@iterator', '__@tag', 'name'] })
      expect(compare(caliper, { k: 'object', fields: ['__@iterator', '__@tag', 'name'], index: false })).toBe(
        'identical',
      )
    })

    it('the name `bigint` is the primitive, which `PropType` has no kind for', () => {
      expect.hasAssertions()
      const key = caliperNorm({
        kind: 'union',
        members: [{ kind: 'string' }, { kind: 'number' }, { kind: 'named', name: 'bigint', args: [] }],
      })
      expect(compare(key, { ...optional(union(prim('string'), prim('number'), prim('bigint'))), alias: 'Key' })).toBe(
        'identical',
      )
      // A name with arguments is a name.
      expect(caliperNorm({ kind: 'named', name: 'bigint', args: [{ kind: 'string' }] })).toStrictEqual({
        k: 'name',
        name: 'bigint',
      })
    })

    it('an index signature keeps its named fields beside it', () => {
      expect.hasAssertions()
      const caliper = caliperNorm({
        kind: 'object',
        fields: [
          { name: 'id', propType: { kind: 'number' }, required: true, description: '' },
          { name: 'key', propType: { kind: 'number' }, required: true, description: '', indexKey: { kind: 'string' } },
        ],
      })
      expect(caliper).toMatchObject({ fields: ['id'], index: true })
      expect(compare(caliper, { k: 'object', fields: ['id'], index: true, indexValue: prim('number') })).toBe(
        'identical',
      )
      expect(compare(caliper, checkerIndex(prim('number')))).toBe('wrong')
    })

    it('an index signature of another value type, or against a type with none, is still wrong', () => {
      expect.hasAssertions()
      const caliper = caliperNorm(indexSignature('id', { kind: 'number' }))
      expect(compare(caliper, checkerIndex(prim('string')))).toBe('wrong')
      expect(compare(caliper, { k: 'object', fields: [], index: false })).toBe('wrong')
      // A checker index signature Caliper lost reads less precise, as before.
      expect(compare({ k: 'object', fields: [], index: false }, checkerIndex(prim('number')))).toBe('less precise')
    })

    it('a readonly array is the array, as the checker reads it', () => {
      expect.hasAssertions()
      // chakra's `readonly T[]`, shadcn's `readonly Preset[]`.
      const readonlyStrings = caliperNorm({ kind: 'readonly', element: { kind: 'array', element: { kind: 'string' } } })
      expect(readonlyStrings).toStrictEqual({ k: 'array', element: prim('string') })
      expect(compare(readonlyStrings, { k: 'array', element: prim('string') })).toBe('identical')
      const readonlyTuple = caliperNorm({
        kind: 'readonly',
        element: { kind: 'tuple', elements: [{ type: { kind: 'string' } }, { type: { kind: 'number' } }] },
      })
      expect(compare(readonlyTuple, { k: 'tuple', elements: [prim('string'), prim('number')] })).toBe('identical')
    })

    it('the kinds schema version 5 adds read as the checker states them', () => {
      expect.hasAssertions()
      const string = { kind: 'string' }
      const number = { kind: 'number' }
      // A rest element is its element type, as the checker's tuple type arguments are.
      const rest = caliperNorm({
        kind: 'tuple',
        elements: [
          { type: string, label: 'head' },
          { type: number, label: 'tail', modifier: 'rest' },
        ],
      })
      expect(rest).toStrictEqual({ k: 'tuple', elements: [prim('string'), prim('number')] })
      expect(caliperNorm({ kind: 'bigintLiteral', value: '-10' })).toStrictEqual(lit('-10', 'bigint'))
      expect(
        caliperNorm({
          kind: 'enumMember',
          enum: 'Size',
          member: 'Small',
          value: { kind: 'stringLiteral', value: 'small' },
        }),
      ).toStrictEqual(lit('small', 'string'))
      expect(caliperNorm({ kind: 'templateLiteral', parts: [{ type: number }, { text: 'px' }] })).toStrictEqual(
        prim('string'),
      )
      expect(caliperNorm({ kind: 'recursive', name: 'Json', args: [] })).toStrictEqual({ k: 'name', name: 'Json' })
      expect(caliperNorm({ kind: 'truncated', text: 'A', limit: { type: 'depth' } })).toStrictEqual({ k: 'opaque' })
      const call = { typeParams: [], params: [{ name: 'value', type: string }], returnType: string }
      const construct = { construct: true, typeParams: [], params: [], returnType: string }
      // Overloads read as the first call signature, a callable with properties as its properties, and construct
      // signatures alone as an empty object: what the checker side makes of each.
      expect(caliperNorm({ kind: 'callable', signatures: [construct, call, call], fields: [] })).toStrictEqual({
        k: 'function',
        params: [{ optional: false, rest: false, type: prim('string') }],
        returns: prim('string'),
      })
      const field = { name: 'displayName', propType: string, required: false, description: '' }
      expect(caliperNorm({ kind: 'callable', signatures: [call], fields: [field] })).toStrictEqual({
        k: 'object',
        fields: ['displayName'],
        index: false,
      })
      expect(caliperNorm({ kind: 'callable', signatures: [construct], fields: [] })).toStrictEqual({
        k: 'object',
        fields: [],
        index: false,
      })
    })

    it('a readonly array of another element type is still wrong', () => {
      expect.hasAssertions()
      const readonlyNumbers = caliperNorm({ kind: 'readonly', element: { kind: 'array', element: { kind: 'number' } } })
      expect(compare(readonlyNumbers, { k: 'array', element: prim('string') })).toBe('wrong')
      expect(compare(caliperNorm({ kind: 'number' }), prim('string'))).toBe('wrong')
    })

    it('literals beside the string that covers them are spelled when the checker spells them out', () => {
      expect.hasAssertions()
      // Mantine's `variant?: AccordionVariant | (string & {})`: the checker keeps the literals, Caliper reads
      // `'default' | … | string`. Both hold every string.
      const spelledOut = optional(union(lit('default'), lit('filled'), prim('string')))
      const caliper = caliperNorm({
        kind: 'union',
        members: [
          { kind: 'stringLiteral', value: 'default' },
          { kind: 'stringLiteral', value: 'filled' },
          { kind: 'string' },
        ],
      })
      expect(compare(caliper, spelledOut)).toBe('spelled')
      expect(compare(prim('string'), spelledOut)).toBe('spelled')
    })

    it('a string where the checker has only literals is still less precise', () => {
      expect.hasAssertions()
      // No `string` in the checker's union: it admits fewer values than Caliper's.
      expect(compare(prim('string'), optional(union(lit('default'), lit('filled'))))).toBe('less precise')
      expect(compare(union(lit('default'), prim('string')), optional(union(lit('default'), lit('filled'))))).toBe(
        'less precise',
      )
      // A different primitive is not covered by the checker's.
      expect(compare(prim('number'), optional(union(lit('default'), prim('string'))))).toBe('wrong')
    })
  })

  describe('rows from the saved weekly oracle runs', () => {
    // `normalized` forms copied verbatim from the `wrong` examples of the branch's own weekly run (antd, mantine, shadcn,
    // chakra). Each scored `wrong` there; whether it should is decided below.
    const rows: Record<string, { normalized: { caliper: Norm; checker: Norm } }> = JSON.parse(
      readFileSync(join(import.meta.dirname, 'fixtures', 'comparator-artifact-rows.json'), 'utf8'),
    )
    const verdict = (name: string) => compare(rows[name].normalized.caliper, rows[name].normalized.checker)

    it.each(['Badge.color', 'Flex.gap', 'Col.flex', 'Input.type'])(
      "%s: `string & Record<never, never>` absorbs the literals beside it, as the checker's `string` does",
      (name) => {
        expect.hasAssertions()
        expect(verdict(name)).toBe('less precise')
      },
    )

    it.each(['Avatar.color', 'BlockImage.placeholder', 'ThemeProvider.attribute'])(
      '%s: a union with a name or template literal Caliper did not expand is less precise, not wrong',
      (name) => {
        expect.hasAssertions()
        expect(verdict(name)).toBe('less precise')
      },
    )

    it.each(['Form.watch', 'Form.getValues', 'AbsoluteCenter.axis'])(
      '%s: a real difference in the values stays wrong',
      (name) => {
        expect.hasAssertions()
        expect(verdict(name)).toBe('wrong')
      },
    )

    it('`string & {}` absorbs literals like `string & Record<never, never>`', () => {
      expect.hasAssertions()
      const empty: Norm = { k: 'object', fields: [], index: false }
      const literalUnion = union(lit('a'), lit('b'), { k: 'intersection', members: [prim('string'), empty] })
      expect(compare(literalUnion, optional(union(lit('a'), lit('b'), prim('string'))))).toBe('less precise')
    })

    it('a name beside a literal does not hide a different primitive', () => {
      expect.hasAssertions()
      // `number` is not `string`, whatever names the union holds.
      expect(compare(prim('number'), optional(prim('string')))).toBe('wrong')
      expect(compare(union(prim('number'), lit('a')), optional(prim('string')))).toBe('wrong')
    })
  })

  describe('an index-only object and a string literal are told from `{}` and a number', () => {
    const idx = (value: object) => ({
      kind: 'object',
      fields: [{ name: 'key', propType: value, required: true, description: '', indexKey: { kind: 'string' } }],
    })
    const checkerIndex = (value: Norm): Norm => ({ k: 'object', fields: [], index: true, indexValue: value })

    it('a string literal beside `number` is not the number it spells', () => {
      expect.hasAssertions()
      // `"0" | number`: the checker keeps the string `"0"`, which `number` does not cover.
      const checker = optional(union(lit('0', 'string'), prim('number')))
      expect(compare(prim('number'), checker)).toBe('less precise')
      // Nor does a `"0" | number` Caliper reports hold only the checker's `number`.
      expect(compare(union(lit('0', 'string'), prim('number')), optional(prim('number')))).toBe('wrong')
      // A string literal and a number literal of the same text are two values.
      expect(compare(lit('0', 'string'), lit('0', 'number'))).toBe('wrong')
    })

    it('`"true"` is a string literal, covered by `string`, and `true` is not', () => {
      expect.hasAssertions()
      expect(compare(prim('string'), optional(union(lit('true', 'string'), prim('string'))))).toBe('spelled')
      expect(compare(lit('true', 'string'), lit('true', 'boolean'))).toBe('wrong')
      // `true | false` is still boolean, and `"true" | "false"` is not.
      expect(compare(prim('boolean'), union(lit('true'), lit('false')))).toBe('identical')
      expect(compare(prim('boolean'), union(lit('true', 'string'), lit('false', 'string')))).not.toBe('identical')
    })

    it('a literal a primitive of its kind covers still adds nothing', () => {
      expect.hasAssertions()
      expect(compare(prim('number'), optional(union(lit('0', 'number'), prim('number'))))).toBe('spelled')
      expect(compare(prim('string'), optional(union(lit('0', 'string'), prim('string'))))).toBe('spelled')
    })

    it('an index-only object beside a named type is not dropped as `{}`', () => {
      expect.hasAssertions()
      // Mantine's `labelProps?: React.ComponentPropsWithoutRef<'label'> & { [key: string]: any }`.
      const caliper = caliperNorm({
        kind: 'intersection',
        members: [{ kind: 'named', name: 'ComponentPropsWithoutRef' }, idx({ kind: 'any' })],
      })
      expect(caliper.k).toBe('intersection')
      const checker: Norm = { k: 'intersection', members: [{ k: 'opaque' }, checkerIndex(prim('any'))] }
      expect(compare(caliper, optional(checker))).toBe('identical')
      // An empty object is still dropped.
      const emptied = caliperNorm({
        kind: 'intersection',
        members: [{ kind: 'string' }, { kind: 'object', fields: [] }],
      })
      expect(emptied).toStrictEqual(prim('string'))
    })

    it('an index signature stated without its value type is less precise, not identical', () => {
      expect.hasAssertions()
      const noValue: Norm = { k: 'object', fields: [], index: true }
      expect(compare(noValue, checkerIndex(prim('number')))).toBe('less precise')
      expect(compare(caliperNorm(idx({ kind: 'number' })), noValue)).toBe('less precise')
    })
  })
})
