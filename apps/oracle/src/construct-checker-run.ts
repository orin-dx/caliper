// One run of the construct suite through the checker path (ADR 0026), shared by the lane script
// (src/construct-checker-lane.ts) and its test (tests/construct-checker-lane.test.ts): `caliper-docgen extract
// --experimental-semantic-types` with a `tsrs` binary over apps/oracle/construct-suite, and each row's verdict against
// its golden and against checker-expectations.json.
import { execFileSync } from 'node:child_process'
import { readFileSync, realpathSync } from 'node:fs'
import { join } from 'node:path'
import { checkCaliperShape, flattenProps } from './caliper-input.ts'
import {
  expectationFailure,
  expectationProblems,
  listFixtures,
  PROP,
  rowVerdict,
  type Expectation,
  type Golden,
  type RowVerdict,
} from './construct-suite.ts'

export const suiteDir = realpathSync(join(import.meta.dirname, '..', 'construct-suite'))

export interface Row {
  id: string
  verdict: RowVerdict
  /** The structural path's allowed verdicts (expectations.json), for the improvements listing. */
  structural: RowVerdict[]
  /** The checker answered for this row; false means it kept its structural result, so the verdict says nothing about the checker. */
  answered: boolean
  type: unknown
  degradedBy: unknown
  golden: string
}

export interface LaneRun {
  elapsedMs: number
  rows: Row[]
  /** A message when the checker backend was unavailable, in which case no row is meaningful. */
  backendProblem?: string
}

interface Expectations {
  rows: Record<string, Expectation>
}

function readExpectations(name: string): Expectations {
  return JSON.parse(readFileSync(join(suiteDir, name), 'utf8'))
}

export const structuralExpectations = (): Expectations => readExpectations('expectations.json')
export const checkerExpectations = (): Expectations => readExpectations('checker-expectations.json')

function extract(bin: string, extra: string[]): any {
  const stdout = execFileSync(
    bin,
    [
      'extract',
      '--src',
      suiteDir,
      '--json',
      '--html-attributes',
      'full',
      '--include-unexported',
      '--load-dependencies',
      '--collapse-inherited-props',
      'none',
      '--no-cache',
      '--quiet',
      ...extra,
    ],
    { encoding: 'utf8', maxBuffer: 512 * 1024 * 1024 },
  )
  const caliper = JSON.parse(stdout)
  checkCaliperShape(caliper)
  return caliper
}

/** Run the suite through the checker path and give every row its verdict. */
export function runCheckerLane(bin: string, tsrs: string): LaneRun {
  const started = performance.now()
  const checked = extract(bin, ['--experimental-semantic-types', '--tsc-path', tsrs])
  const elapsedMs = Math.round(performance.now() - started)

  const backendProblem = (checked.diagnostics ?? []).find((d: any) =>
    String(d.message).includes('checker backend unavailable'),
  )?.message

  /** The component keys the checker answered for, from the provenance diagnostic the merge writes. */
  const answered = new Set<string>()
  for (const diagnostic of checked.diagnostics ?? []) {
    if (String(diagnostic.message).includes('resolved by the TypeScript checker')) {
      // `affects` is a list of component keys, or the id of one in `nameLists` (the canonical JSON shares repeated lists).
      const affects =
        typeof diagnostic.affects === 'string' ? checked.nameLists[diagnostic.affects] : diagnostic.affects
      for (const key of affects ?? []) answered.add(key)
    }
  }

  const byFile = new Map<string, { key: string; prop: any }>()
  for (const [key, component] of Object.entries<any>(checked.components)) {
    if (component.displayName !== 'Row') continue
    const id = flattenProps(checked, key, component)[PROP]
    byFile.set(realpathSync(component.filePath), { key, prop: id === undefined ? undefined : checked.props[id] })
  }

  const structural = structuralExpectations().rows
  const rows: Row[] = []
  for (const fixture of listFixtures(suiteDir)) {
    const golden: Golden = JSON.parse(readFileSync(fixture.golden, 'utf8'))
    const found = byFile.get(realpathSync(fixture.file))
    rows.push({
      id: fixture.id,
      verdict: rowVerdict(found?.prop, golden),
      structural: structural[fixture.id]?.verdicts ?? [],
      answered: found !== undefined && answered.has(found.key),
      type: found?.prop?.type,
      degradedBy: found?.prop?.degradedBy,
      golden: golden.text,
    })
  }
  return { elapsedMs, rows, backendProblem }
}

/** What is wrong with the run against checker-expectations.json: rows the checker did not answer for, rows better or
 * worse than allowed, and expectations that do not match the fixtures. Empty when the lane holds. */
export function laneFailures(run: LaneRun, expectations: Expectations): string[] {
  const failures = expectationProblems(
    run.rows.map((row) => row.id),
    expectations.rows,
  )
  for (const row of run.rows) {
    if (!row.answered) failures.push(`${row.id}: the checker did not answer for this row (kept the structural result)`)
    const expectation = expectations.rows[row.id]
    const failure = expectation && expectationFailure(row.id, row.verdict, expectation)
    if (failure) failures.push(failure)
  }
  return failures
}
