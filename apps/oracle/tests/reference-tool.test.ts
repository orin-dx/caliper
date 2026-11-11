// A tool's canonical JSON as the reference reads it: the type a prop has, and whether a collapsed layer names it.
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { prim } from '../src/oracle-compare.ts'
import { loadTool, type Tool } from '../src/reference-tool.ts'
import { SUPPORTED_SCHEMA_VERSION } from '../src/schema-version.ts'

let dir: string
let tool: Tool

beforeAll(() => {
  dir = realpathSync(mkdtempSync(join(tmpdir(), 'reference-tool-')))
  const file = join(dir, 'Button.tsx')
  writeFileSync(file, '')
  writeFileSync(
    join(dir, 'caliper.json'),
    JSON.stringify({
      schemaVersion: SUPPORTED_SCHEMA_VERSION,
      components: {
        Button: {
          displayName: 'Button',
          filePath: file,
          exports: [{ name: 'Button', members: [] }],
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
        Packed: {
          displayName: 'Packed',
          filePath: file,
          exports: [{ name: 'Packed', members: [] }],
          props: ['set1'],
          notableInherited: ['setN'],
        },
        // Two components the module exports under one name cannot be told apart, so neither is the tool's answer.
        'Twin (a)': { displayName: 'Twin', filePath: file, exports: [{ name: 'Twin', members: [] }], props: {} },
        'Twin (b)': { displayName: 'Twin', filePath: file, exports: [{ name: 'Twin', members: [] }], props: {} },
        // A component no module exports is nobody's `Hidden`, and a sub-component is reached through its parent's export.
        Hidden: { displayName: 'Hidden', filePath: file, exports: [], props: {} },
        Inner: { displayName: 'Inner', filePath: file, exports: [{ name: 'Button', members: ['Inner'] }], props: {} },
        // The export is the name the module gives it, not the component's own.
        _Card: { displayName: '_Card', filePath: file, exports: [{ name: 'Card', members: [] }], props: ['set1'] },
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
    expect(tool.lookup('Button.tsx', ['Button'], 'count')).toStrictEqual({
      found: true,
      norm: prim('number'),
      layered: false,
    })
  })

  it('gives the type of a curated prop the component inherits, and the component’s own prop wins a clash', () => {
    expect.hasAssertions()
    expect(tool.lookup('Button.tsx', ['Button'], 'notable')).toStrictEqual({
      found: true,
      norm: prim('string'),
      layered: false,
    })
    // `count` is in both: the own `number`, not the notable `boolean`.
    expect(tool.lookup('Button.tsx', ['Button'], 'count').norm).toStrictEqual(prim('number'))
    // The same, in schema 2's array form.
    expect(tool.lookup('Button.tsx', ['Packed'], 'notable').norm).toStrictEqual(prim('string'))
  })

  it('reads a collapsed layer’s interned member list, and an inline one, as surfacing the prop', () => {
    expect.hasAssertions()
    expect(tool.lookup('Button.tsx', ['Button'], 'onCopy')).toStrictEqual({
      found: true,
      norm: undefined,
      layered: true,
    })
    expect(tool.lookup('Button.tsx', ['Button'], 'legacyProp')).toStrictEqual({
      found: true,
      norm: undefined,
      layered: true,
    })
  })

  it('does not take part of a list’s id for a member, and calls an unnamed prop neither typed nor layered', () => {
    expect.hasAssertions()
    expect(tool.lookup('Button.tsx', ['Button'], 'list')).toStrictEqual({
      found: true,
      norm: undefined,
      layered: false,
    })
    expect(tool.lookup('Button.tsx', ['Button'], 'nothing')).toStrictEqual({
      found: true,
      norm: undefined,
      layered: false,
    })
  })

  it('has no single component when the name is absent or ambiguous in the file', () => {
    expect.hasAssertions()
    expect(tool.has('Button.tsx', ['Button'])).toBe(true)
    expect(tool.has('Button.tsx', ['Twin'])).toBe(false)
    expect(tool.has('Button.tsx', ['Missing'])).toBe(false)
    expect(tool.has('Other.tsx', ['Button'])).toBe(false)
    expect(tool.lookup('Button.tsx', ['Twin'], 'x')).toStrictEqual({ found: false, layered: false })
  })

  it('finds a component by the name its module exports it under, and no unexported or attached one', () => {
    expect.hasAssertions()
    expect(tool.lookup('Button.tsx', ['Card'], 'count')).toStrictEqual({
      found: true,
      norm: prim('number'),
      layered: false,
    })
    expect(tool.has('Button.tsx', ['_Card'])).toBe(false)
    expect(tool.has('Button.tsx', ['Hidden'])).toBe(false)
    expect(tool.has('Button.tsx', ['Inner'])).toBe(false)
    expect(tool.has('Button.tsx', ['default', 'Card'])).toBe(true)
  })

  it('stops on output of a version that does not list exports', () => {
    expect.hasAssertions()
    writeFileSync(join(dir, 'old.json'), JSON.stringify({ schemaVersion: 6, components: {}, propSets: {}, props: {} }))
    expect(() => loadTool(join(dir, 'old.json'), dir)).toThrow(
      `caliper output has schemaVersion 6, the oracle reads ${SUPPORTED_SCHEMA_VERSION}`,
    )
  })
})
