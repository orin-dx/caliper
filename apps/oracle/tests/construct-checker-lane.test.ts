// The construct suite through the checker path (ADR 0026; construct-suite/README.md "The checker lane"): every row's
// verdict held to checker-expectations.json, so a row that regresses fails and a row that improves fails until the
// expectation is updated. The live run needs Caliper's release binary, which links the checker; without it, it skips with
// a warning, and REQUIRE_CALIPER_BIN=1 turns the skip into a failure.
import { existsSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  againstStructural,
  checkerExpectations,
  fallbackCause,
  fallbackFailure,
  fallbackWorklist,
  laneFailures,
  runCheckerLane,
  structuralExpectations,
  type LaneRun,
} from '../src/construct-checker-run.ts'
import { expectationProblems } from '../src/construct-suite.ts'

const bin = resolve(
  process.env.CALIPER_BIN ?? join(import.meta.dirname, '..', '..', '..', 'target', 'release', 'caliper-docgen'),
)
const binAvailable = existsSync(bin)
if (!binAvailable) console.warn(`checker lane skipped: no caliper binary at ${bin}. Set REQUIRE_CALIPER_BIN=1 to fail.`)

const row = (id: string, verdict: LaneRun['rows'][number]['verdict'], answered = true): LaneRun['rows'][number] => ({
  id,
  verdict,
  structural: [],
  answered,
  type: undefined,
  degradedBy: undefined,
  golden: '',
  facts: [],
  fallback: null,
})

describe('the fallback-off column (ADR 0026 S7b)', () => {
  it('says how a row changes with the fallback off, and nothing for one that reads the same', () => {
    expect.hasAssertions()
    const on = { ...row('a/01', 'identical'), type: { kind: 'string' }, degradedBy: ['indexedAccess'] }
    expect(fallbackCause(on, { ...on })).toBeNull()
    expect(fallbackCause(on, undefined)).toBe('no checker answer')
    expect(fallbackCause(on, { ...on, answered: false })).toBe('no checker answer')
    const opaque = { ...on, verdict: 'opaque with marker' as const, type: { kind: 'opaque' } }
    expect(fallbackCause(on, opaque)).toBe('checker could not read: indexedAccess')
    expect(fallbackCause(on, { ...on, facts: ['description' as const] })).toBe('structural description')
    expect(fallbackCause(on, { ...on, verdict: 'wrong', type: { kind: 'number' }, degradedBy: undefined })).toBe(
      'type: wrong',
    )
  })

  it('fails a row whose dependence on the fallback is unlisted, listed wrongly, or listed and gone', () => {
    expect.hasAssertions()
    const green = { verdicts: ['identical', 'spelled'] } satisfies Parameters<typeof fallbackFailure>[2]
    const listed = { ...green, fallback: 'checker could not read: indexedAccess' }
    expect(fallbackFailure('a/01', null, green)).toBeNull()
    expect(fallbackFailure('a/01', 'checker could not read: indexedAccess', listed)).toBeNull()
    expect(fallbackFailure('a/01', 'checker could not read: indexedAccess', green)).toContain('says nothing')
    expect(fallbackFailure('a/01', 'no checker answer', listed)).toContain('leans on the structural')
    expect(fallbackFailure('a/01', null, listed)).toContain('remove its "fallback" entry')
  })

  it('groups the rows that lean on the fallback by cause', () => {
    expect.hasAssertions()
    const rows = [
      { ...row('a/01', 'identical'), fallback: 'checker could not read: indexedAccess' },
      row('a/02', 'identical'),
      { ...row('a/03', 'identical'), fallback: 'checker could not read: indexedAccess' },
    ]
    expect([...fallbackWorklist(rows)]).toStrictEqual([['checker could not read: indexedAccess', ['a/01', 'a/03']]])
  })
})

describe('the checker lane expectations', () => {
  it('cover exactly the rows of the structural expectations, and every gap has a reason', () => {
    expect.hasAssertions()
    const ids = Object.keys(structuralExpectations().rows)
    expect(expectationProblems(ids, checkerExpectations().rows)).toStrictEqual([])
  })

  it('rank no row below the structural path except a listed improvement (ADR 0026 S4)', () => {
    expect.hasAssertions()
    expect(againstStructural(structuralExpectations().rows, checkerExpectations().rows)).toStrictEqual([])
  })

  it('fail a checker row ranked below the structural one without a reason, and a reason on a row that is not', () => {
    expect.hasAssertions()
    const structural = {
      'a/01-green': { verdicts: ['identical', 'spelled'] },
      'a/02-gap': { verdicts: ['less precise'], gap: 'unevaluated' },
      'a/03-green': { verdicts: ['identical', 'spelled'] },
      'a/04-gap': { verdicts: ['less precise'], gap: 'unevaluated' },
    } satisfies Parameters<typeof againstStructural>[0]
    const checker = {
      'a/01-green': { verdicts: ['less precise'], gap: 'expanded' },
      'a/02-gap': { verdicts: ['identical', 'spelled'] },
      'a/03-green': { verdicts: ['less precise'], gap: 'expanded', improvement: 'states more than the name' },
      'a/04-gap': { verdicts: ['less precise'], gap: 'unevaluated', improvement: 'stale' },
    } satisfies Parameters<typeof againstStructural>[1]
    const failures = againstStructural(structural, checker)
    expect(failures).toHaveLength(2)
    expect(failures[0]).toContain('a/01-green: the checker path allows less precise, worse than')
    expect(failures[1]).toContain('a/04-gap: "improvement" is listed, but the checker path is not ranked below')
  })

  it('fail a checker row that may state a fact differently where the structural path states it right', () => {
    expect.hasAssertions()
    const structural = {
      'a/01-green': { verdicts: ['identical', 'spelled'] },
      'a/02-green': { verdicts: ['identical', 'spelled'], facts: { order: 'kept as a literal union' } },
    } satisfies Parameters<typeof againstStructural>[0]
    const checker = {
      'a/01-green': { verdicts: ['identical', 'spelled'], facts: { parent: 'not supplied' } },
      'a/02-green': { verdicts: ['identical', 'spelled'], facts: { order: 'sorted' } },
    } satisfies Parameters<typeof againstStructural>[1]
    expect(againstStructural(structural, checker)).toStrictEqual([
      'a/01-green: the checker path may state parent differently, where the structural path states it right',
    ])
  })

  it('fail a run with a row whose facts differ from the golden unless its expectation lists them', () => {
    expect.hasAssertions()
    const rows = {
      'a/01-green': { verdicts: ['identical', 'spelled'] },
    } satisfies Parameters<typeof laneFailures>[1]['rows']
    const run: LaneRun = { elapsedMs: 0, rows: [{ ...row('a/01-green', 'identical'), facts: ['tags', 'parent'] }] }
    expect(laneFailures(run, { rows })).toStrictEqual([
      'a/01-green: tags differs from the golden (a regression, or list it under "facts" with the reason)',
      'a/01-green: parent differs from the golden (a regression, or list it under "facts" with the reason)',
    ])
  })

  it('fail a run with a row better or worse than allowed, or one the checker did not answer for', () => {
    expect.hasAssertions()
    const rows = {
      'a/01-green': { verdicts: ['identical', 'spelled'] },
      'a/02-gap': { verdicts: ['less precise'], gap: 'unevaluated' },
      'a/03-green': { verdicts: ['identical', 'spelled'] },
    } satisfies Parameters<typeof laneFailures>[1]['rows']
    const ok: LaneRun = {
      elapsedMs: 0,
      rows: [row('a/01-green', 'spelled'), row('a/02-gap', 'less precise'), row('a/03-green', 'identical')],
    }
    expect(laneFailures(ok, { rows })).toStrictEqual([])
    const moved: LaneRun = {
      elapsedMs: 0,
      rows: [row('a/01-green', 'wrong'), row('a/02-gap', 'identical'), row('a/03-green', 'identical', false)],
    }
    const failures = laneFailures(moved, { rows })
    expect(failures).toHaveLength(3)
    expect(failures.join('\n')).toContain('a/01-green: wrong (worse than expected')
    expect(failures.join('\n')).toContain('a/02-gap: identical (better than expected')
    expect(failures.join('\n')).toContain('a/03-green: the checker did not answer')
  })
})

describe('the live checker lane', () => {
  it.runIf(process.env.REQUIRE_CALIPER_BIN === '1')('has the caliper binary when REQUIRE_CALIPER_BIN=1', () => {
    expect(binAvailable ? 'present' : `REQUIRE_CALIPER_BIN=1 but there is no caliper binary at ${bin}`).toBe('present')
  })

  it.skipIf(!binAvailable)('gives every row the verdict checker-expectations.json allows', () => {
    expect.hasAssertions()
    const run = runCheckerLane(bin)
    expect(run.backendProblem).toBeUndefined()
    expect(run.rows.length).toBeGreaterThan(0)
    expect(laneFailures(run, checkerExpectations())).toStrictEqual([])
  })
})
