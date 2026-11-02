// The whole reference on a small corpus built here: the real checker and the real parser judge each prop, and a
// hand-written Caliper output is scored against them.
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { Row } from '../src/reference-report.ts'

const SOURCE = `export type Mode = 'a' | 'b'
export interface ButtonProps {
  size?: 'small' | 'large'
  count: number
  mode: Mode
  onChange(value: string): void
  items: Array<string>
}
export function Button(props: ButtonProps) { return null }
export function Merged(props: ButtonProps & { id: string }) { return null }
export function Picked(props: Pick<ButtonProps, 'count'>) { return null }
export function Generic<T>(props: { value: T; render: (item: T) => string }) { return null }
`

const TSCONFIG = {
  include: ['src/**/*.ts', 'src/**/*.tsx'],
  compilerOptions: {
    target: 'ES2022',
    module: 'ESNext',
    moduleResolution: 'bundler',
    strict: true,
    skipLibCheck: true,
    noEmit: true,
  },
}

let dir: string
let rows: Row[]
let report: any
let again: string
let reported: Row[]

function run(out: string, extra: Record<string, unknown> = {}): void {
  const config = join(dir, 'config.json')
  writeFileSync(
    config,
    JSON.stringify({
      ...extra,
      corpora: [
        {
          name: 'fixture',
          root: dir,
          tsconfig: join(dir, 'tsconfig.json'),
          files: join(dir, 'files.txt'),
          caliper: join(dir, 'caliper.json'),
        },
      ],
      perCorpus: 100,
      seed: 'test',
      out,
    }),
  )
  execFileSync(process.execPath, [join(import.meta.dirname, '..', 'src', 'reference.ts'), '--config', config], {
    stdio: 'pipe',
  })
}

const find = (component: string, prop: string) => rows.find((r) => r.component === component && r.prop === prop)!

beforeAll(() => {
  dir = realpathSync(mkdtempSync(join(tmpdir(), 'reference-')))
  mkdirSync(join(dir, 'src'))
  const file = join(dir, 'src', 'Button.tsx')
  writeFileSync(file, SOURCE)
  writeFileSync(join(dir, 'tsconfig.json'), JSON.stringify(TSCONFIG))
  writeFileSync(join(dir, 'files.txt'), `${file}\n`)
  const component = (name: string, props: Record<string, string>, members: string[] = []) => ({
    displayName: name,
    filePath: file,
    props,
    inheritance: members.length > 0 ? [{ typeName: 'ButtonProps', members }] : [],
  })
  writeFileSync(
    join(dir, 'caliper.json'),
    JSON.stringify({
      components: {
        Button: component('Button', { size: 'size', count: 'count', mode: 'mode' }, ['items']),
        Merged: component('Merged', { count: 'count' }),
        Picked: component('Picked', { count: 'text' }),
      },
      props: {
        size: { type: { kind: 'literalUnion', members: ['small', 'large'] } },
        count: { type: { kind: 'number' } },
        mode: { type: { kind: 'named', name: 'Mode' } },
        text: { type: { kind: 'string' } },
      },
    }),
  )
  run(join(dir, 'report.json'), { allComponents: true })
  report = JSON.parse(readFileSync(join(dir, 'report.json'), 'utf8'))
  rows = JSON.parse(readFileSync(join(dir, 'report.json.rows.json'), 'utf8'))
  run(join(dir, 'again.json'), { allComponents: true })
  again = readFileSync(join(dir, 'again.json.rows.json'), 'utf8')
  run(join(dir, 'reported.json'))
  reported = JSON.parse(readFileSync(join(dir, 'reported.json.rows.json'), 'utf8'))
}, 120_000)

afterAll(() => rmSync(dir, { recursive: true, force: true }))

describe('the reference on a fixture corpus', () => {
  it('judges every prop of every exported component, and is the same on a second run', () => {
    expect.hasAssertions()
    expect(rows).toHaveLength(14)
    expect(again).toBe(JSON.stringify(rows, null, 1))
  })

  it('has the checker and the declaration agree on a plain, a literal, a named and a method prop', () => {
    expect.hasAssertions()
    for (const prop of ['count', 'size', 'mode', 'onChange']) {
      expect(find('Button', prop)).toMatchObject({ reference: 'agreed', basis: 'both' })
    }
    expect(find('Button', 'count')).toMatchObject({ declared: 'number', stratum: 'own' })
    expect(find('Button', 'size')).toMatchObject({ declared: "'small' | 'large'", stratum: 'union_literal' })
    expect(find('Button', 'onChange')).toMatchObject({ declared: '(value: string) => void', stratum: 'function' })
  })

  it('sorts props into strata by how the props type is built and what the prop is', () => {
    expect.hasAssertions()
    expect(find('Merged', 'id').stratum).toBe('intersection')
    expect(find('Picked', 'count').stratum).toBe('mapped')
    expect(find('Generic', 'value').stratum).toBe('generic')
    expect(find('Generic', 'render').stratum).toBe('function')
  })

  it('scores Caliper: correct where it matches, wrong where it contradicts, a layer member as surfaced', () => {
    expect.hasAssertions()
    expect(find('Button', 'count').tool).toStrictEqual({ outcome: 'correct', basis: 'both' })
    expect(find('Button', 'size').tool).toStrictEqual({ outcome: 'correct', basis: 'both' })
    expect(find('Button', 'mode').tool).toStrictEqual({ outcome: 'correct', basis: 'both' })
    expect(find('Picked', 'count').tool).toStrictEqual({ outcome: 'wrong', basis: 'both' })
    expect(find('Button', 'items').tool).toStrictEqual({ outcome: 'layer' })
    expect(find('Button', 'onChange').tool).toStrictEqual({ outcome: 'missing' })
    expect(find('Merged', 'id').tool).toStrictEqual({ outcome: 'missing' })
  })

  it('leaves a prop of a component Caliper does not report unscored, and says how many', () => {
    expect.hasAssertions()
    expect(find('Generic', 'value').tool).toBeUndefined()
    expect(report.fixture.scored.tool_unmatched).toBe(2)
    expect(report.fixture.scored.props).toBe(12)
  })

  it('draws the control sample only from the components the tool reports, unless told to take them all', () => {
    expect.hasAssertions()
    expect(reported).toHaveLength(12)
    expect(reported.some((r) => r.component === 'Generic')).toBe(false)
    expect(reported.every((r) => r.tool !== undefined)).toBe(true)
  })

  it('counts the outcomes and finds nothing contested', () => {
    expect.hasAssertions()
    expect(report.fixture.reference.contested).toBe(0)
    expect(report.fixture.scored.outcomes).toStrictEqual({
      correct: 4,
      'less precise': 0,
      wrong: 1,
      contested: 0,
      'no reference': 0,
      layer: 1,
      missing: 6,
    })
  })
})
