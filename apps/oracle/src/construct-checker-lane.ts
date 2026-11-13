// The construct suite through the checker path (ADR 0026): `caliper-docgen extract` (the checker is on by default) over
// apps/oracle/construct-suite, and each row's verdict against its golden. It is a gate on
// construct-suite/checker-expectations.json: it prints the report, then exits 1 when a row is better or worse than the
// expectation allows (an improvement means the gap closed: update the file), when the checker did not answer for a row,
// when the checker backend was unavailable, or when checker-expectations.json ranks a row below expectations.json
// without an `improvement` reason (ADR 0026's S4 gate). Run it with `just test-checker-lane`, or by hand:
//
//   cargo build --release -p caliper-docgen
//   CALIPER_SEMANTIC=1 pnpm --filter caliper-docgen-oracle construct:checker
//
// A row the checker did not answer for (it kept the structural result) is listed apart: its verdict says nothing about
// the checker. `--json <file>` also writes every row with the type each side gave.
import { existsSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import {
  againstStructural,
  checkerExpectations,
  laneFailures,
  runCheckerLane,
  structuralExpectations,
  type Row,
} from './construct-checker-run.ts'
import { isAgreement } from './vocabulary.ts'

if (process.env.CALIPER_SEMANTIC !== '1') {
  console.error('construct-checker-lane: set CALIPER_SEMANTIC=1 to run the suite through the checker.')
  process.exit(2)
}
const bin = resolve(
  process.env.CALIPER_BIN ?? join(import.meta.dirname, '..', '..', '..', 'target', 'release', 'caliper-docgen'),
)
if (!existsSync(bin)) {
  console.error(`construct-checker-lane: no caliper binary at ${bin}; build one or set CALIPER_BIN.`)
  process.exit(2)
}
const jsonOut = process.argv.includes('--json') ? process.argv[process.argv.indexOf('--json') + 1] : undefined

const run = runCheckerLane(bin)
if (run.backendProblem !== undefined) {
  console.error(`construct-checker-lane: the checker was unavailable: ${run.backendProblem}`)
  process.exit(1)
}
const { elapsedMs, rows } = run

const checkerRows = rows.filter((row) => row.answered)
const structuralRows = rows.filter((row) => !row.answered)
const green = (row: Row) => isAgreement(row.verdict)
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
for (const row of checkerRows.filter((r) => green(r) && !r.structural.every((v) => isAgreement(v)))) {
  console.log(`  ${row.id}: ${row.verdict} (structural: ${row.structural.join('/')})`)
}
console.log('')
console.log('Rows whose facts differ from the golden (description, tags, default, required, parent, member order):')
for (const row of rows.filter((r) => r.facts.length > 0)) console.log(`  ${row.id}: ${row.facts.join(', ')}`)
console.log('')
console.log('Rows that kept their structural result (no checker verdict):')
for (const row of structuralRows) console.log(`  ${row.id}: ${row.verdict}`)

if (jsonOut) writeFileSync(jsonOut, JSON.stringify({ elapsedMs, rows }, null, 2))

const failures = [
  ...laneFailures(run, checkerExpectations()),
  ...againstStructural(structuralExpectations().rows, checkerExpectations().rows),
]
if (failures.length > 0) {
  console.error('')
  console.error(
    `construct-checker-lane: ${failures.length} row(s) differ from checker-expectations.json or fall below the structural path:`,
  )
  for (const failure of failures) console.error(`  ${failure}`)
  process.exit(1)
}
console.log('')
console.log('every row matches checker-expectations.json')
