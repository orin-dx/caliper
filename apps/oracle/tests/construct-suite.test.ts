// The deterministic construct suite (apps/oracle/construct-suite/README.md): Caliper's prop `p` of every fixture against
// the golden TypeScript 7.1's checker gave it, held to the verdict expectations.json allows. The binary is Caliper's own,
// so without one the suite skips (and fails instead when REQUIRE_CALIPER_BIN=1, as CI sets); the pure helpers are tested either way.
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, realpathSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import type { PropType } from '../../../packages/napi/index.d.ts'
import { checkCaliperShape, flattenProps } from '../src/caliper-input.ts'
import {
  componentsOf,
  expectationFailure,
  expectationProblems,
  factDifferences,
  factFailures,
  listFixtures,
  PROP,
  rowKey,
  rowKeysOf,
  rowVerdict,
  type Expectation,
  type Golden,
} from '../src/construct-suite.ts'
import { prim } from '../src/oracle-compare.ts'
import { lit } from './lit.ts'

const here = import.meta.dirname
const suite = realpathSync(join(here, '..', 'construct-suite'))
const bin = resolve(process.env.CALIPER_BIN ?? join(here, '..', '..', '..', 'target', 'release', 'caliper-docgen'))
const available = existsSync(bin)
const required = process.env.REQUIRE_CALIPER_BIN === '1'
if (!available && !required) {
  console.warn(
    `construct suite skipped: no caliper binary at ${bin}. Build one (cargo build --release -p caliper-docgen) ` +
      'or point CALIPER_BIN at it.',
  )
}

const noFacts: Golden['facts'] = { description: '', tags: {}, default: null, required: true, parent: null, order: null }
const opaque: PropType = { kind: 'opaque', raw: 'T extends U ? X : Y', reason: { type: 'conditionalType' } }
const goldenOf = (checker: Golden['checker']): Golden => ({ checker, text: '', facts: noFacts })
const expectations: { rows: Record<string, Expectation> } = JSON.parse(
  readFileSync(join(suite, 'expectations.json'), 'utf8'),
)

describe('the construct suite helpers', () => {
  it('a degraded prop that is only less precise is an opaque type with a marker', () => {
    expect.hasAssertions()
    const golden = goldenOf(prim('string'))
    expect(rowVerdict({ type: { kind: 'string' } }, golden)).toBe('identical')
    expect(rowVerdict({ type: opaque }, golden)).toBe('less precise')
    expect(rowVerdict({ type: opaque, degradedBy: ['conditionalType'] }, golden)).toBe('opaque with marker')
    expect(rowVerdict({ type: opaque, degradedBy: [] }, golden)).toBe('less precise')
  })

  it('a golden the checker stopped short of is not judged, and ranks below every verdict that judged', () => {
    expect.hasAssertions()
    const golden = goldenOf({ k: 'opaque' })
    expect(rowVerdict({ type: { kind: 'string' } }, golden)).toBe('not judged')
    expect(rowVerdict({ type: opaque, degradedBy: ['conditionalType'] }, golden)).toBe('not judged')
    expect(expectationFailure('r', 'not judged', { verdicts: ['wrong'], gap: 'g' })).toContain('worse than expected')
    expect(
      expectationProblems(['r'], { r: { verdicts: ['not judged'], gap: 'the golden has no type' } }),
    ).toStrictEqual([])
  })

  it('a marker does not excuse a wrong type, and a missing prop is its own verdict', () => {
    expect.hasAssertions()
    const golden = goldenOf(prim('string'))
    expect(rowVerdict({ type: { kind: 'number' }, degradedBy: ['conditionalType'] }, golden)).toBe('wrong')
    expect(rowVerdict(undefined, golden)).toBe('missing')
    expect(rowVerdict({ type: { kind: 'stringLiteral', value: 'a' } }, goldenOf(lit('a', 'string')))).toBe('identical')
  })

  it('the expectations must cover the fixtures, with a reason on every gap and no gap that hides an improvement', () => {
    expect.hasAssertions()
    const rows: Record<string, Expectation> = {
      'a/01-ok': { verdicts: ['identical', 'spelled'] },
      'a/02-gap': { verdicts: ['less precise'], gap: 'unevaluated' },
      'a/03-no-reason': { verdicts: ['wrong'] },
      'a/04-hides': { verdicts: ['less precise', 'identical'], gap: 'x' },
      'a/05-green-with-gap': { verdicts: ['identical'], gap: 'x' },
      'a/99-orphan': { verdicts: ['identical'] },
    }
    const ids = ['a/01-ok', 'a/02-gap', 'a/03-no-reason', 'a/04-hides', 'a/05-green-with-gap', 'a/06-unlisted']
    const problems = expectationProblems(ids, rows)
    expect(problems).toHaveLength(5)
    expect(problems.join('\n')).toContain('a/03-no-reason: a row that is not identical or spelled')
    expect(problems.join('\n')).toContain('a/04-hides: a gap must not allow')
    expect(problems.join('\n')).toContain('a/05-green-with-gap: a green row carries no "gap"')
    expect(problems.join('\n')).toContain('a/06-unlisted: no entry')
    expect(problems.join('\n')).toContain('a/99-orphan: expectation without a fixture')
  })

  it('a row better or worse than its expectation fails, and says which way', () => {
    expect.hasAssertions()
    const gap: Expectation = { verdicts: ['less precise'], gap: 'unevaluated' }
    const green: Expectation = { verdicts: ['identical', 'spelled'] }
    expect(expectationFailure('r', 'less precise', gap)).toBeNull()
    expect(expectationFailure('r', 'spelled', green)).toBeNull()
    expect(expectationFailure('r', 'identical', gap)).toContain('better than expected')
    expect(expectationFailure('r', 'opaque with marker', gap)).toContain('better than expected')
    expect(expectationFailure('r', 'wrong', gap)).toContain('worse than expected')
    expect(expectationFailure('r', 'less precise', green)).toContain('worse than expected')
  })

  it('a fact Caliper states differently from the golden is named, and a union out of written order is one', () => {
    expect.hasAssertions()
    const file = join(suite, 'seed', '01-string-literal-union.tsx')
    const golden: Golden = {
      checker: prim('string'),
      text: '',
      facts: {
        description: 'How large.',
        tags: { since: '2.0' },
        default: "'sm'",
        required: false,
        parent: { name: 'Props', file: 'seed/01-string-literal-union.tsx' },
        order: [lit('b', 'string'), lit('a', 'string')],
      },
    }
    const union = (...values: string[]) => ({
      kind: 'union',
      members: values.map((value) => ({ kind: 'stringLiteral', value })),
    })
    const prop = {
      type: union('b', 'x', 'a'),
      description: 'How large.',
      tags: { since: '2.0' },
      defaultValue: { value: "'sm'", computed: false },
      required: false,
      parent: { name: 'Props', fileName: file },
    }
    expect(factDifferences(prop, golden, suite)).toStrictEqual([])
    expect(
      factDifferences(
        {
          ...prop,
          type: union('a', 'b'),
          description: '',
          tags: {},
          defaultValue: null,
          required: true,
          parent: null,
        },
        golden,
        suite,
      ),
    ).toStrictEqual(['description', 'tags', 'default', 'required', 'parent', 'order'])
    expect(factDifferences({ ...prop, parent: { name: 'Base', fileName: file } }, golden, suite)).toStrictEqual([
      'parent',
    ])
    expect(factDifferences(undefined, golden, suite)).toStrictEqual([])
  })

  it('each member of a union states one written member, so two written members of one name are two places', () => {
    expect.hasAssertions()
    // react/37: `Element | React.JSX.Element | null`, both objects named `Element` on the checker side.
    const element = (fields: string[]): Golden['checker'] => ({ k: 'object', fields, index: false, alias: 'Element' })
    const golden: Golden = {
      checker: prim('null'),
      text: '',
      facts: { ...noFacts, order: [element(['tagName']), element(['key', 'props', 'type']), prim('null')] },
    }
    const named = (name: string) => ({ kind: 'named', name, args: [] })
    const prop = (...members: object[]) => ({
      type: { kind: 'union', members },
      description: '',
      tags: {},
      defaultValue: null,
      required: true,
      parent: null,
    })
    expect(
      factDifferences(prop(named('Element'), named('JSX.Element'), { kind: 'null' }), golden, suite),
    ).toStrictEqual([])
    expect(
      factDifferences(prop({ kind: 'null' }, named('Element'), named('JSX.Element')), golden, suite),
    ).toStrictEqual(['order'])
  })

  it('a differing fact fails unless listed, and a listed fact that agrees fails as stale', () => {
    expect.hasAssertions()
    const row: Expectation = { verdicts: ['identical', 'spelled'], facts: { order: 'the checker sorts members' } }
    expect(factFailures('r', ['order'], row)).toStrictEqual([])
    expect(factFailures('r', ['parent', 'order'], row)).toStrictEqual([
      'r: parent differs from the golden (a regression, or list it under "facts" with the reason)',
    ])
    expect(factFailures('r', [], row)).toStrictEqual([
      'r: order no longer differs from the golden: remove it from "facts"',
    ])
    const unknownFact: Record<string, Expectation> = JSON.parse(
      '{"r":{"verdicts":["identical"],"facts":{"colour":"x"}}}',
    )
    expect(expectationProblems(['r'], unknownFact)).toStrictEqual(['r: unknown fact colour'])
  })

  it('the expectations file agrees with the fixtures on disk', () => {
    expect.hasAssertions()
    const ids = listFixtures(suite).map((f) => f.id)
    expect(expectationProblems(ids, expectations.rows)).toStrictEqual([])
  })

  it('a fixture of several components lists each with its export path, and a plain fixture is one Row', () => {
    expect.hasAssertions()
    expect(componentsOf('// components: Item=default.Item Menu=default\nfunction Item() {}')).toStrictEqual([
      { component: 'Item', exportPath: ['default', 'Item'] },
      { component: 'Menu', exportPath: ['default'] },
    ])
    expect(componentsOf('export function Row(props: Props) { return null }')).toBeUndefined()
    expect(() => componentsOf('// components: Item')).toThrow('<display name>=<export path>')
    const rows = listFixtures(suite).filter((f) => f.file.endsWith('03-attached-to-default.tsx'))
    expect(
      rows.map((f) => [f.id, f.component, f.exportPath, f.golden.slice(f.golden.lastIndexOf('/') + 1)]),
    ).toStrictEqual([
      ['exports/03-attached-to-default#Item', 'Item', ['default', 'Item'], '03-attached-to-default.Item.golden.json'],
      ['exports/03-attached-to-default#Menu', 'Menu', ['default'], '03-attached-to-default.Menu.golden.json'],
    ])
  })

  it('every fixture has a golden', () => {
    expect.hasAssertions()
    const missing = listFixtures(suite).filter((f) => !existsSync(f.golden))
    expect(missing.map((f) => f.id)).toStrictEqual([])
  })
})

it.runIf(required)('the caliper binary REQUIRE_CALIPER_BIN=1 demands exists', () => {
  expect(available, `REQUIRE_CALIPER_BIN=1 but there is no caliper binary at ${bin}`).toBe(true)
})

describe.skipIf(!available)('the construct suite against the caliper binary', () => {
  it('every row has the verdict expectations.json allows', () => {
    expect.hasAssertions()
    // The addon's options (docs/measurement.md), with a cold cache so the run does not depend on one. The structural
    // path: expectations.json is the resolver's, and the checker lane holds the checker's.
    const stdout = execFileSync(
      bin,
      [
        'extract',
        '--src',
        suite,
        '--json',
        '--html-attributes',
        'full',
        '--include-unexported',
        '--load-dependencies',
        '--collapse-inherited-props',
        'none',
        '--no-cache',
        '--quiet',
        '--no-checker',
      ],
      { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 },
    )
    const caliper = JSON.parse(stdout)
    checkCaliperShape(caliper)
    const propByRow = new Map<string, any>()
    for (const [key, component] of Object.entries<any>(caliper.components)) {
      const id = flattenProps(caliper, key, component)[PROP]
      for (const row of rowKeysOf(component)) propByRow.set(row, id === undefined ? undefined : caliper.props[id])
    }

    const failures: string[] = []
    for (const fixture of listFixtures(suite)) {
      const golden: Golden = JSON.parse(readFileSync(fixture.golden, 'utf8'))
      const prop = propByRow.get(rowKey(realpathSync(fixture.file), fixture.exportPath))
      const verdict = rowVerdict(prop, golden)
      const row = expectations.rows[fixture.id]
      const failure = row && expectationFailure(fixture.id, verdict, row)
      if (failure) failures.push(failure)
      if (row) failures.push(...factFailures(fixture.id, factDifferences(prop, golden, suite), row))
    }
    expect(failures).toStrictEqual([])
  })
})
