// The construct suite through the checker path (ADR 0026; construct-suite/README.md "The checker lane"): every row's
// verdict held to checker-expectations.json, so a row that regresses fails and a row that improves fails until the
// expectation is updated. The live run needs Caliper's release binary and the pinned tsrs; without them it skips with a
// warning, and REQUIRE_CALIPER_BIN=1 / REQUIRE_TSRS=1 turn each skip into a failure.
import { existsSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  againstStructural,
  checkerExpectations,
  laneFailures,
  runCheckerLane,
  structuralExpectations,
  type LaneRun,
} from '../src/construct-checker-run.ts'
import { expectationProblems } from '../src/construct-suite.ts'
import { resolveTsrsBin } from '../src/tsrs-bin.ts'

const bin = resolve(
  process.env.CALIPER_BIN ?? join(import.meta.dirname, '..', '..', '..', 'target', 'release', 'caliper-docgen'),
)
const binAvailable = existsSync(bin)
let tsrs: string | undefined
let tsrsProblem = ''
try {
  tsrs = await resolveTsrsBin()
  if (!existsSync(tsrs)) {
    tsrsProblem = `no tsrs binary at ${tsrs}`
    tsrs = undefined
  }
} catch (error) {
  tsrsProblem = error instanceof Error ? error.message : String(error)
}
if (!binAvailable) console.warn(`checker lane skipped: no caliper binary at ${bin}. Set REQUIRE_CALIPER_BIN=1 to fail.`)
if (tsrs === undefined) console.warn(`checker lane skipped: ${tsrsProblem}. Set REQUIRE_TSRS=1 to fail.`)

const row = (id: string, verdict: LaneRun['rows'][number]['verdict'], answered = true): LaneRun['rows'][number] => ({
  id,
  verdict,
  structural: [],
  answered,
  type: undefined,
  degradedBy: undefined,
  golden: '',
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

  it.runIf(process.env.REQUIRE_TSRS === '1')('has the tsrs binary when REQUIRE_TSRS=1', () => {
    expect(tsrs === undefined ? `REQUIRE_TSRS=1 but ${tsrsProblem}` : 'present').toBe('present')
  })

  it.skipIf(!binAvailable || tsrs === undefined)('gives every row the verdict checker-expectations.json allows', () => {
    expect.hasAssertions()
    const run = runCheckerLane(bin, tsrs!)
    expect(run.backendProblem).toBeUndefined()
    expect(run.rows.length).toBeGreaterThan(0)
    expect(laneFailures(run, checkerExpectations())).toStrictEqual([])
  })
})
