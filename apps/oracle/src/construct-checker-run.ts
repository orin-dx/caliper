// One run of the construct suite through the checker path (ADR 0026), shared by the lane script
// (src/construct-checker-lane.ts) and its test (tests/construct-checker-lane.test.ts): `caliper-docgen extract
// --no-resolver-facts` (the checker is on by default, the `tsrs` crates linked into it) over apps/oracle/construct-suite,
// and each row's verdict against its golden and against checker-expectations.json.
import { execFileSync } from 'node:child_process'
import { readFileSync, realpathSync } from 'node:fs'
import { join } from 'node:path'
import { checkCaliperShape, flattenProps } from './caliper-input.ts'
import {
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
  type FactName,
  type Golden,
} from './construct-suite.ts'
import { ROW_VERDICTS, type RowVerdict } from './vocabulary.ts'

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
  /** The facts that differ from the golden's (`factDifferences`). */
  facts: FactName[]
  /** Null when the row passes with the structural fallback off (the checker and oxc's syntax alone gave its prop);
   * otherwise what the structural answer still supplies (`fallbackCause`). */
  fallback: string | null
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b)

/** How a row leans on the structural fallback (ADR 0026, S7b): null when the row reads the same with the fallback off
 * (`extract --no-structural-fallback`) as on, else what changed when it went: `no checker answer` (the component's
 * props became opaque), `checker could not read: <kinds>` (the checker's own `opaque` type replaced the structural one),
 * `structural <facts>` (a description or tags only the structural pass had), else `type: <verdict with it off>`. */
export function fallbackCause(on: LaneRow, off: LaneRow | undefined): string | null {
  if (off === undefined || !off.answered) return 'no checker answer'
  const changed = [...new Set([...on.facts, ...off.facts])].filter(
    (fact) => on.facts.includes(fact) !== off.facts.includes(fact),
  )
  if (same(on.type, off.type) && same(on.degradedBy, off.degradedBy) && changed.length === 0) return null
  const kinds = (Array.isArray(off.degradedBy) ? off.degradedBy : []).filter((kind) => kind !== 'truncated')
  if (!same(on.type, off.type) && kinds.length > 0) return `checker could not read: ${kinds.join(', ')}`
  if (changed.length > 0) return `structural ${changed.join(', ')}`
  return `type: ${off.verdict}`
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
export function runCheckerLane(bin: string): LaneRun {
  const started = performance.now()
  // The resolver's answer is not read for the facts oxc's syntax supplies (ADR 0026, S7a2): the lane holds the checker
  // and the syntax alone to them.
  const checked = extract(bin, ['--no-resolver-facts'])
  const elapsedMs = Math.round(performance.now() - started)
  // The same run with the structural fallback off (S7b): the fallback-off column.
  const off = new Map(laneRows(extract(bin, ['--no-structural-fallback'])).map((row) => [row.id, row]))

  const backendProblem = (checked.diagnostics ?? []).find((d: any) =>
    String(d.message).includes('checker backend unavailable'),
  )?.message
  const rows: Row[] = laneRows(checked).map((row) =>
    Object.assign(row, { fallback: fallbackCause(row, off.get(row.id)) }),
  )
  return { elapsedMs, rows, backendProblem }
}

/** A row as one run states it, before the fallback-off column. */
type LaneRow = Omit<Row, 'fallback'>

/** Every row's verdict in one `extract` output. */
function laneRows(checked: any): LaneRow[] {
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

  const byRow = new Map<string, { key: string; prop: any }>()
  for (const [key, component] of Object.entries<any>(checked.components)) {
    const id = flattenProps(checked, key, component)[PROP]
    for (const row of rowKeysOf(component)) {
      byRow.set(row, { key, prop: id === undefined ? undefined : checked.props[id] })
    }
  }

  const structural = structuralExpectations().rows
  return listFixtures(suiteDir).map((fixture) => {
    const golden: Golden = JSON.parse(readFileSync(fixture.golden, 'utf8'))
    const found = byRow.get(rowKey(realpathSync(fixture.file), fixture.exportPath))
    return {
      id: fixture.id,
      verdict: rowVerdict(found?.prop, golden),
      structural: structural[fixture.id]?.verdicts ?? [],
      answered: found !== undefined && answered.has(found.key),
      type: found?.prop?.type,
      degradedBy: found?.prop?.degradedBy,
      golden: golden.text,
      facts: factDifferences(found?.prop, golden, suiteDir),
    }
  })
}

/** The worst verdict an expectation allows, as a rank in ROW_VERDICTS (higher is worse). */
const worstAllowed = (row: Expectation) => Math.max(...row.verdicts.map((v) => ROW_VERDICTS.indexOf(v)))

/** ADR 0026's S4 gate, from the two expectation files alone: the checker path equals the structural path on every row
 * except listed improvements. A row whose checker verdicts rank below the structural ones is a regression unless its
 * checker expectation gives an `improvement` reason (an answer the comparison scores lower although it states more,
 * such as an expansion where the structural pass kept a name). An `improvement` on a row the checker does not rank
 * below the structural path is stale. Rows the checker ranks above the structural path need no entry: the structural
 * expectation's `gap` already says what the structural path misses. A fact (description, tags, default, required,
 * parent, member order) the checker path may state differently while the structural path states it as the checker does
 * is a regression too (S7a: the checker supplies them before the resolver goes). Empty when the gate holds. */
export function againstStructural(
  structural: Record<string, Expectation>,
  checker: Record<string, Expectation>,
): string[] {
  const failures: string[] = []
  for (const [id, row] of Object.entries(checker)) {
    const base = structural[id]
    if (base === undefined) continue
    const worse = worstAllowed(row) > worstAllowed(base)
    const reason = typeof row.improvement === 'string' && row.improvement.trim() !== ''
    if (worse && !reason) {
      failures.push(
        `${id}: the checker path allows ${row.verdicts.join(' | ')}, worse than the structural path's ${base.verdicts.join(' | ')}: a regression unless listed with an "improvement" reason`,
      )
    }
    if (!worse && row.improvement !== undefined) {
      failures.push(
        `${id}: "improvement" is listed, but the checker path is not ranked below the structural path: remove it`,
      )
    }
    for (const fact of Object.keys(row.facts ?? {})) {
      if (base.facts?.[fact as FactName] === undefined) {
        failures.push(
          `${id}: the checker path may state ${fact} differently, where the structural path states it right`,
        )
      }
    }
  }
  return failures
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
    if (expectation) failures.push(...factFailures(row.id, row.facts, expectation))
    const drift = expectation && fallbackFailure(row.id, row.fallback, expectation)
    if (drift) failures.push(drift)
  }
  return failures
}

/** Null when the row's `fallback` entry says what the run found: absent for a row that passes with the fallback off,
 * the cause for one that leans on it. Otherwise what moved: an unlisted dependence is a regression, a listed one that
 * went is worklist progress to record by removing the entry. */
export function fallbackFailure(id: string, cause: string | null, row: Expectation): string | null {
  if (cause === null && row.fallback === undefined) return null
  if (cause === null) return `${id}: passes with the structural fallback off now: remove its "fallback" entry`
  if (row.fallback !== cause) {
    return `${id}: leans on the structural fallback (${cause}); "fallback" says ${row.fallback === undefined ? 'nothing' : `"${row.fallback}"`}`
  }
  return null
}

/** The rows that lean on the structural fallback, by cause: the S7c to S7e worklist. */
export function fallbackWorklist(rows: Row[]): Map<string, string[]> {
  const byCause = new Map<string, string[]>()
  for (const row of rows) {
    if (row.fallback !== null) byCause.set(row.fallback, [...(byCause.get(row.fallback) ?? []), row.id])
  }
  return byCause
}
