// The deterministic construct suite's pure half (apps/oracle/construct-suite/README.md): which fixtures exist, the verdict
// one of Caliper's props earns against its golden, and the check of the expectations file. Nothing here runs the checker
// or Caliper, so tests/construct-suite.test.ts can pin it without either.
import { readdirSync } from 'node:fs'
import { join } from 'node:path'
import { caliperNorm } from './caliper-norm.ts'
import { compare, type Norm, type Verdict } from './oracle-compare.ts'

/** The prop every fixture states its type through. */
export const PROP = 'p'

/** What the suite says about a row: the comparison's verdict, plus the two cases it cannot make. `opaque with marker`
 * is a type Caliper left unevaluated and said so (`degradedBy`); `missing` is a prop Caliper did not list. */
export type RowVerdict = Verdict | 'opaque with marker' | 'missing'

export const GREEN: RowVerdict[] = ['identical', 'spelled']

/** Best to worst. A row whose verdict moved down this list got worse; up, better. `checker any/error` is a fixture
 * problem (the golden has no type), ranked last so it never reads as an improvement. */
export const ORDER: RowVerdict[] = [
  'identical',
  'spelled',
  'opaque with marker',
  'less precise',
  'wrong',
  'missing',
  'checker any/error',
]

/** What `--check` and the test read back from `<name>.golden.json`. */
export interface Golden {
  /** The checker's type of prop `p`, normalized by src/oracle-checker.ts. */
  checker: Norm
  /** `checker.typeToString` of the same type, for the reader of a diff. */
  text: string
}

export interface Fixture {
  /** `<family>/<NN>-<slug>`: the row id the expectations file keys on. */
  id: string
  /** The fixture's absolute path. */
  file: string
  /** The golden's absolute path, next to the fixture. */
  golden: string
}

const FIXTURE = /^\d{2}-[a-z0-9][a-z0-9-]*\.tsx$/

/** Every `<family>/<NN>-<slug>.tsx` under `dir`, sorted by id. A `.tsx` whose name breaks the pattern is an error, not a
 * row that quietly never runs. */
export function listFixtures(dir: string): Fixture[] {
  const fixtures: Fixture[] = []
  for (const family of readdirSync(dir, { withFileTypes: true })) {
    if (!family.isDirectory()) continue
    for (const entry of readdirSync(join(dir, family.name))) {
      if (!entry.endsWith('.tsx')) continue
      if (!FIXTURE.test(entry)) {
        throw new Error(`${family.name}/${entry}: a fixture is named <NN>-<slug>.tsx (lowercase)`)
      }
      const file = join(dir, family.name, entry)
      fixtures.push({
        id: `${family.name}/${entry.replace(/\.tsx$/, '')}`,
        file,
        golden: file.replace(/\.tsx$/, '.golden.json'),
      })
    }
  }
  return fixtures.toSorted((a, b) => (a.id < b.id ? -1 : 1))
}

/** The verdict for Caliper's prop against the golden. A prop with `degradedBy` that is only `less precise` is an
 * `opaque with marker`: Caliper said it did not evaluate the type. A `wrong` prop stays wrong with or without a marker. */
export function rowVerdict(prop: { type: unknown; degradedBy?: unknown } | undefined, golden: Golden): RowVerdict {
  if (prop === undefined) return 'missing'
  const verdict = compare(caliperNorm(prop.type), golden.checker)
  const marked = Array.isArray(prop.degradedBy) && prop.degradedBy.length > 0
  return marked && verdict === 'less precise' ? 'opaque with marker' : verdict
}

export interface Expectation {
  /** The verdicts the row may have; the test fails on any other. */
  verdicts: RowVerdict[]
  /** Why Caliper falls short of the checker here. Present exactly on a documented gap. */
  gap?: string
  /** checker-expectations.json only: why the checker path's answer is accepted although its verdict ranks below the
   * structural path's for the same row (see `againstStructural` in src/construct-checker-run.ts). */
  improvement?: string
}

/** Problems with the expectations against the fixtures; empty when they agree. */
export function expectationProblems(ids: string[], rows: Record<string, Expectation>): string[] {
  const problems: string[] = []
  for (const id of ids) {
    const row = rows[id]
    if (row === undefined) {
      problems.push(`${id}: no entry in expectations.json`)
      continue
    }
    if (!Array.isArray(row.verdicts) || row.verdicts.length === 0) {
      problems.push(`${id}: "verdicts" must list at least one verdict`)
      continue
    }
    const unknown = row.verdicts.filter((v) => !ORDER.includes(v))
    if (unknown.length > 0) problems.push(`${id}: unknown verdict ${unknown.join(', ')}`)
    const green = row.verdicts.every((v) => GREEN.includes(v))
    const anyGreen = row.verdicts.some((v) => GREEN.includes(v))
    if (green && row.gap !== undefined) problems.push(`${id}: a green row carries no "gap"`)
    if (!green && anyGreen) {
      problems.push(`${id}: a gap must not allow identical or spelled, or an improvement goes unnoticed`)
    }
    if (!anyGreen && (typeof row.gap !== 'string' || row.gap.trim() === '')) {
      problems.push(`${id}: a row that is not identical or spelled is a documented gap and needs a "gap" reason`)
    }
  }
  const known = new Set(ids)
  for (const id of Object.keys(rows)) if (!known.has(id)) problems.push(`${id}: expectation without a fixture`)
  return problems
}

/** Null when `verdict` is allowed; otherwise what moved, for the failure message. */
export function expectationFailure(id: string, verdict: RowVerdict, row: Expectation): string | null {
  if (row.verdicts.includes(verdict)) return null
  const rank = (v: RowVerdict) => ORDER.indexOf(v)
  const here = rank(verdict)
  let direction = 'unexpected'
  if (here < Math.min(...row.verdicts.map(rank))) direction = 'better than expected: update expectations.json'
  else if (here > Math.max(...row.verdicts.map(rank))) direction = 'worse than expected: a regression'
  return `${id}: ${verdict} (${direction}; allowed ${row.verdicts.join(' | ')})`
}
