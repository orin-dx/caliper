// The whole reference on a small corpus built here: the real checker and the real parser judge each prop, and a
// hand-written Caliper output is scored against them.
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { Row } from '../src/reference-report.ts'
import { SUPPORTED_SCHEMA_VERSION } from '../src/schema-version.ts'

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
// Props the checker infers: no declaration of theirs writes a type, so the declaration can say nothing.
const defaults = { count: 1 }
export function Counter(props: typeof defaults) { return null }
const handlers = { tick() { return 1 } }
export function Ticker(props: typeof handlers) { return null }
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
let listedReport: any
let listedRows: Row[]

function run(out: string, extra: Record<string, unknown> = {}): void {
  const config = join(dir, 'config.json')
  writeFileSync(
    config,
    JSON.stringify({
      perCorpus: 100,
      seed: 'test',
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
    exports: [{ name, members: [] }],
    props,
    inheritance: members.length > 0 ? [{ typeName: 'ButtonProps', members }] : [],
  })
  writeFileSync(
    join(dir, 'caliper.json'),
    JSON.stringify({
      schemaVersion: SUPPORTED_SCHEMA_VERSION,
      components: {
        Button: component('Button', { size: 'size', count: 'count', mode: 'mode' }, ['items']),
        Merged: component('Merged', { count: 'count' }),
        Picked: component('Picked', { count: 'text' }),
      },
      propSets: {},
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
  // A list: two props the checker finds (one under its display name), three it cannot, and one for another corpus.
  const at = (name: string, prop: string, over: Record<string, string> = {}) => ({
    corpus: 'fixture',
    file: 'src/Button.tsx',
    component: name,
    prop,
    ...over,
  })
  writeFileSync(
    join(dir, 'list.json'),
    JSON.stringify([
      at('Button', 'count'),
      at('Button (src/Button.tsx)', 'size'),
      at('Missing', 'x'),
      at('Button', 'count', { file: 'src/Other.tsx' }),
      at('Button', 'nope'),
      at('Button', 'count', { corpus: 'elsewhere' }),
    ]),
  )
  run(join(dir, 'listed.json'), { allComponents: true, perCorpus: 0, list: join(dir, 'list.json') })
  listedReport = JSON.parse(readFileSync(join(dir, 'listed.json'), 'utf8'))
  listedRows = JSON.parse(readFileSync(join(dir, 'listed.json.rows.json'), 'utf8'))
}, 120_000)

afterAll(() => rmSync(dir, { recursive: true, force: true }))

describe('the reference on a fixture corpus', () => {
  it('judges every prop of every exported component, and is the same on a second run', () => {
    expect.hasAssertions()
    expect(rows).toHaveLength(16)
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
    expect(report.fixture.scored.tool_unmatched).toBe(4)
    expect(report.fixture.scored.props).toBe(12)
  })

  it('has the declaration abstain for a prop the checker infers, and never contests the checker over it', () => {
    expect.hasAssertions()
    // `const defaults = { count: 1 }`: the declaration `count: 1` is a value, not a literal type annotation.
    expect(find('Counter', 'count')).toMatchObject({
      reference: 'agreed',
      basis: 'checker',
      why: 'abstained',
      abstained: 'unsupported declaration',
      checker: 'number',
    })
    // `tick() { return 1 }` has no return type to read; it is not `() => void`.
    expect(find('Ticker', 'tick')).toMatchObject({
      reference: 'agreed',
      basis: 'checker',
      abstained: 'no annotation',
      checker: '() => number',
    })
  })

  it('judges the listed props the checker finds, normalizing a display name, and reports the ones it cannot find', () => {
    expect.hasAssertions()
    expect(listedRows.map((r) => `${r.component}.${r.prop}:${r.stratum}`).toSorted()).toStrictEqual([
      'Button.count:listed',
      'Button.size:listed',
    ])
    // Six entries, one for another corpus: five are this corpus's, two found.
    expect(listedReport.fixture.listed).toStrictEqual({
      requested: 5,
      found: 2,
      missing: ['src/Button.tsx: Button.nope', 'src/Button.tsx: Missing.x', 'src/Other.tsx: Button.count'],
    })
  })

  it('counts the listed props that name a corpus the run does not have, instead of dropping them', () => {
    expect.hasAssertions()
    expect(listedReport.listed_for_other_corpora).toStrictEqual({ count: 1, corpora: ['elsewhere'] })
    expect(report.listed_for_other_corpora).toStrictEqual({ count: 0, corpora: [] })
  })

  it('reports no list when none was given', () => {
    expect.hasAssertions()
    expect(report.fixture.listed).toStrictEqual({ requested: 0, found: 0, missing: [] })
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
