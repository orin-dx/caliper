// The deterministic construct suite's pure half (apps/oracle/construct-suite/README.md): which fixtures exist, the verdict
// one of Caliper's props earns against its golden, and the check of the expectations file. Nothing here runs the checker
// or Caliper, so tests/construct-suite.test.ts can pin it without either.
import { readdirSync, readFileSync, realpathSync } from 'node:fs'
import { join } from 'node:path'
import { caliperNorm } from './caliper-norm.ts'
import { pathOfExport, exportPathKey, type ComponentExport } from './component-join.ts'
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

/** One row: a component of a fixture file. A file holds one row (`Row`, exported as `Row`) unless it lists its
 * components (`componentsOf`). */
export interface Fixture {
  /** `<family>/<NN>-<slug>`, or `<family>/<NN>-<slug>#<component>` in a file of several components: the row id the
   * expectations file keys on. */
  id: string
  /** The fixture's absolute path. */
  file: string
  /** The golden's absolute path, next to the fixture: `<NN>-<slug>.golden.json`, or `<NN>-<slug>.<component>.golden.json`. */
  golden: string
  /** The component's display name in Caliper's output. */
  component: string
  /** How the module exports it: the export name (`default` for the default export), then the properties of the export
   * a sub-component is attached to (`['default', 'Item']` for `Menu.Item = Item; export default Menu`). */
  exportPath: string[]
}

const FIXTURE = /^\d{2}-[a-z0-9][a-z0-9-]*\.tsx$/
const COMPONENTS = /^\/\/ components:(.*)$/m

/** A fixture of several components lists them in a comment, each as `<display name>=<export path>`:
 * `// components: Item=default.Item Menu=default`. Undefined for a fixture of one `Row`. */
export function componentsOf(source: string): { component: string; exportPath: string[] }[] | undefined {
  const line = COMPONENTS.exec(source)?.[1]
  if (line === undefined) return undefined
  return line
    .trim()
    .split(/\s+/)
    .map((entry) => {
      const [component, path] = entry.split('=')
      if (!component || !path) throw new Error(`"${entry}": a listed component is <display name>=<export path>`)
      return { component, exportPath: path.split('.') }
    })
}

/** Every row of every `<family>/<NN>-<slug>.tsx` under `dir`, sorted by id. A `.tsx` whose name breaks the pattern is
 * an error, not a row that quietly never runs. */
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
      const id = `${family.name}/${entry.replace(/\.tsx$/, '')}`
      const listed = componentsOf(readFileSync(file, 'utf8'))
      if (listed === undefined) {
        fixtures.push({
          id,
          file,
          golden: file.replace(/\.tsx$/, '.golden.json'),
          component: 'Row',
          exportPath: ['Row'],
        })
        continue
      }
      for (const { component, exportPath } of listed) {
        fixtures.push({
          id: `${id}#${component}`,
          file,
          golden: file.replace(/\.tsx$/, `.${component}.golden.json`),
          component,
          exportPath,
        })
      }
    }
  }
  return fixtures.toSorted((a, b) => (a.id < b.id ? -1 : 1))
}

/** Where Caliper's component for a row is, keyed by `rowKey`: the file and the way its module exports it, the same on
 * both sides, never the display name that two components of a file can share. */
export const rowKey = (file: string, exportedAs: readonly string[]): string => exportPathKey(file, exportedAs)

/** The `rowKey` of each way the module exports one of Caliper's components. */
export function rowKeysOf(component: { filePath: string; exports: ComponentExport[] }): string[] {
  const file = realpathSync(component.filePath)
  return component.exports.map((e) => rowKey(file, pathOfExport(e)))
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
