// A tool's canonical JSON as the reference reads it: the type a prop has, and whether a collapsed layer names it.
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { prim } from '../src/oracle-compare.ts'
import { loadTool, type Tool } from '../src/reference-tool.ts'

let dir: string
let tool: Tool

beforeAll(() => {
  dir = realpathSync(mkdtempSync(join(tmpdir(), 'reference-tool-')))
  const file = join(dir, 'Button.tsx')
  writeFileSync(file, '')
  writeFileSync(
    join(dir, 'caliper.json'),
    JSON.stringify({
      components: {
        Button: {
          displayName: 'Button',
          filePath: file,
          // Schema 2: a component lists prop-set ids, each a map of prop name to prop id.
          props: ['set1'],
          // The curated props of a collapsed source, which a props table lists after the component's own. A prop named
          // in both is the component's own.
          notableInherited: { notable: 'p2', count: 'p9' },
          // The members are interned: an id into `nameLists`. An older output lists them inline.
          inheritance: [
            { typeName: 'DOMEvents', members: 'list1' },
            { typeName: 'Legacy', members: ['legacyProp'] },
          ],
        },
        // The same two lists in schema 2's form: arrays of prop-set ids.
        Packed: { displayName: 'Packed', filePath: file, props: ['set1'], notableInherited: ['setN'] },
        'Twin (a)': { displayName: 'Twin', filePath: file, props: {} },
        'Twin (b)': { displayName: 'Twin', filePath: file, props: {} },
      },
      propSets: { set1: { count: 'p1' }, setN: { notable: 'p2' } },
      nameLists: { list1: ['onCopy', 'onCut'] },
      props: {
        p1: { type: { kind: 'number' } },
        p2: { type: { kind: 'string' } },
        p9: { type: { kind: 'boolean' } },
      },
    }),
  )
  tool = loadTool(join(dir, 'caliper.json'), dir)
})

afterAll(() => rmSync(dir, { recursive: true, force: true }))

describe('loadTool', () => {
  it('gives the type of a prop the component lists, through its prop set', () => {
    expect.hasAssertions()
    expect(tool.lookup('Button.tsx', 'Button', 'count')).toStrictEqual({
      found: true,
      norm: prim('number'),
      layered: false,
    })
  })

  it('gives the type of a curated prop the component inherits, and the component’s own prop wins a clash', () => {
    expect.hasAssertions()
    expect(tool.lookup('Button.tsx', 'Button', 'notable')).toStrictEqual({
      found: true,
      norm: prim('string'),
      layered: false,
    })
    // `count` is in both: the own `number`, not the notable `boolean`.
    expect(tool.lookup('Button.tsx', 'Button', 'count').norm).toStrictEqual(prim('number'))
    // The same, in schema 2's array form.
    expect(tool.lookup('Button.tsx', 'Packed', 'notable').norm).toStrictEqual(prim('string'))
  })

  it('reads a collapsed layer’s interned member list, and an inline one, as surfacing the prop', () => {
    expect.hasAssertions()
    expect(tool.lookup('Button.tsx', 'Button', 'onCopy')).toStrictEqual({ found: true, norm: undefined, layered: true })
    expect(tool.lookup('Button.tsx', 'Button', 'legacyProp')).toStrictEqual({
      found: true,
      norm: undefined,
      layered: true,
    })
  })

  it('does not take part of a list’s id for a member, and calls an unnamed prop neither typed nor layered', () => {
    expect.hasAssertions()
    expect(tool.lookup('Button.tsx', 'Button', 'list')).toStrictEqual({ found: true, norm: undefined, layered: false })
    expect(tool.lookup('Button.tsx', 'Button', 'nothing')).toStrictEqual({
      found: true,
      norm: undefined,
      layered: false,
    })
  })

  it('has no single component when the name is absent or ambiguous in the file', () => {
    expect.hasAssertions()
    expect(tool.has('Button.tsx', 'Button')).toBe(true)
    expect(tool.has('Button.tsx', 'Twin')).toBe(false)
    expect(tool.has('Button.tsx', 'Missing')).toBe(false)
    expect(tool.has('Other.tsx', 'Button')).toBe(false)
    expect(tool.lookup('Button.tsx', 'Twin', 'x')).toStrictEqual({ found: false, layered: false })
  })
})
