// The shape rules that say a function is no component. scripts/weekly.py `exclusion` applies the same rules to RDT's
// entries; both read src/component-shape.json and both run the cases of tests/fixtures/component-shape-cases.json
// (scripts/test_component_shape.py), so they cannot drift apart.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { nonComponentReason, type ShapeProp } from '../src/component-shape.ts'

interface Case {
  why: string
  name: string
  file: string
  props: Record<string, { parent?: string; type: string }>
  expected: string | null
}

const cases: Case[] = JSON.parse(
  readFileSync(join(import.meta.dirname, 'fixtures', 'component-shape-cases.json'), 'utf8'),
)

const shapeProps = (props: Case['props']) =>
  new Map<string, ShapeProp>(Object.entries(props).map(([name, p]) => [name, { path: p.parent, type: () => p.type }]))

describe('nonComponentReason', () => {
  it.each(cases)('$why ($name)', ({ name, file, props, expected }) => {
    expect(nonComponentReason(name, file, shapeProps(props)) ?? null).toBe(expected)
  })

  it('reads a prop type only when the rules before it leave the function undecided', () => {
    expect.hasAssertions()
    let read = 0
    const props = new Map<string, ShapeProp>([
      [
        'x',
        {
          path: '/r/src/a.tsx',
          type: () => {
            read++
            return 'string'
          },
        },
      ],
    ])

    expect(nonComponentReason('useA', '/r/src/a.tsx', props)).toBe('hook')
    expect(read).toBe(0)
    expect(nonComponentReason('A', '/r/src/a.tsx', props)).toBeUndefined()
    expect(read).toBe(1)
  })
})
