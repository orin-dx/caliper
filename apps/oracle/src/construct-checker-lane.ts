// The construct suite through the checker path (ADR 0026): `caliper-docgen extract --experimental-semantic-types` with a
// `tsrs` binary over apps/oracle/construct-suite, and each row's verdict against its golden. It is evidence for the
// slice that makes the checker the default (S4), not a gate: it prints, it does not fail on a row, and it is not a CI
// job. Run it by hand:
//
//   cargo build --release -p caliper-docgen
//   CALIPER_SEMANTIC=1 TSRS_BIN=<node_modules/@tsrs/<platform>/tsrs> pnpm --filter caliper-docgen-oracle construct:checker
//
// A row the checker did not answer for (it kept the structural result) is listed apart: its verdict says nothing about
// the checker. `--json <file>` also writes every row with the type each side gave.
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { checkCaliperShape, flattenProps } from './caliper-input.ts'
import {
  GREEN,
  listFixtures,
  PROP,
  rowVerdict,
  type Expectation,
  type Golden,
  type RowVerdict,
} from './construct-suite.ts'

if (process.env.CALIPER_SEMANTIC !== '1') {
  console.error('construct-checker-lane: set CALIPER_SEMANTIC=1 (and TSRS_BIN) to run the suite through the checker.')
  process.exit(2)
}
const tsrs = process.env.TSRS_BIN
if (!tsrs || !existsSync(tsrs)) {
  console.error(`construct-checker-lane: TSRS_BIN must name a tsrs binary (got ${JSON.stringify(tsrs)}).`)
  process.exit(2)
}
const bin = resolve(
  process.env.CALIPER_BIN ?? join(import.meta.dirname, '..', '..', '..', 'target', 'release', 'caliper-docgen'),
)
if (!existsSync(bin)) {
  console.error(`construct-checker-lane: no caliper binary at ${bin}; build one or set CALIPER_BIN.`)
  process.exit(2)
}

const suite = realpathSync(join(import.meta.dirname, '..', 'construct-suite'))
const expectations: { rows: Record<string, Expectation> } = JSON.parse(
  readFileSync(join(suite, 'expectations.json'), 'utf8'),
)
const jsonOut = process.argv.includes('--json') ? process.argv[process.argv.indexOf('--json') + 1] : undefined

function extract(extra: string[]): any {
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
      ...extra,
    ],
    { encoding: 'utf8', maxBuffer: 512 * 1024 * 1024 },
  )
  const caliper = JSON.parse(stdout)
  checkCaliperShape(caliper)
  return caliper
}

const started = performance.now()
const checked = extract(['--experimental-semantic-types', '--tsc-path', tsrs])
const elapsedMs = Math.round(performance.now() - started)

/** The component keys the checker answered for, from the provenance diagnostic the merge writes. */
const answered = new Set<string>()
for (const diagnostic of checked.diagnostics ?? []) {
  if (String(diagnostic.message).includes('resolved by the TypeScript checker')) {
    // `affects` is a list of component keys, or the id of one in `nameLists` (the canonical JSON shares repeated lists).
    const affects = typeof diagnostic.affects === 'string' ? checked.nameLists[diagnostic.affects] : diagnostic.affects
    for (const key of affects ?? []) answered.add(key)
  }
}
const backendProblems = (checked.diagnostics ?? []).filter((d: any) =>
  String(d.message).includes('checker backend unavailable'),
)
if (backendProblems.length > 0) {
  console.error(`construct-checker-lane: the checker was unavailable: ${backendProblems[0].message}`)
  process.exit(1)
}

interface Row {
  id: string
  verdict: RowVerdict
  structural: RowVerdict[]
  answered: boolean
  type: unknown
  degradedBy: unknown
  golden: string
}

const byFile = new Map<string, { key: string; prop: any }>()
for (const [key, component] of Object.entries<any>(checked.components)) {
  if (component.displayName !== 'Row') continue
  const id = flattenProps(checked, key, component)[PROP]
  byFile.set(realpathSync(component.filePath), { key, prop: id === undefined ? undefined : checked.props[id] })
}

const rows: Row[] = []
for (const fixture of listFixtures(suite)) {
  const golden: Golden = JSON.parse(readFileSync(fixture.golden, 'utf8'))
  const found = byFile.get(realpathSync(fixture.file))
  rows.push({
    id: fixture.id,
    verdict: rowVerdict(found?.prop, golden),
    structural: expectations.rows[fixture.id]?.verdicts ?? [],
    answered: found !== undefined && answered.has(found.key),
    type: found?.prop?.type,
    degradedBy: found?.prop?.degradedBy,
    golden: golden.text,
  })
}

const checkerRows = rows.filter((row) => row.answered)
const structuralRows = rows.filter((row) => !row.answered)
const green = (row: Row) => GREEN.includes(row.verdict)
console.log(
  `checker path over ${rows.length} rows in ${elapsedMs} ms: ${checkerRows.length} answered by the checker, ${structuralRows.length} kept their structural result`,
)
const count = (list: Row[]) => {
  const tally = new Map<string, number>()
  for (const row of list) tally.set(row.verdict, (tally.get(row.verdict) ?? 0) + 1)
  return [...tally].map(([verdict, n]) => `${verdict} ${n}`).join(', ')
}
console.log(`  checker rows: ${count(checkerRows)}`)
console.log('')
console.log('Rows the checker answered that are not identical or spelled (verdict | structural expectation | golden):')
for (const row of checkerRows.filter((r) => !green(r))) {
  console.log(`  ${row.id}: ${row.verdict} | ${row.structural.join('/')} | ${row.golden}`)
}
console.log('')
console.log('Rows the checker answered that the structural result did not get green (improvements):')
for (const row of checkerRows.filter((r) => green(r) && !r.structural.every((v) => GREEN.includes(v)))) {
  console.log(`  ${row.id}: ${row.verdict} (structural: ${row.structural.join('/')})`)
}
console.log('')
console.log('Rows that kept their structural result (no checker verdict):')
for (const row of structuralRows) console.log(`  ${row.id}: ${row.verdict}`)

if (jsonOut) writeFileSync(jsonOut, JSON.stringify({ elapsedMs, rows }, null, 2))
