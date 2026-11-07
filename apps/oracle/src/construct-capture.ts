// Writes (or, with --check, verifies) the golden files of the deterministic construct suite: for every fixture under
// construct-suite/, the type TypeScript 7.1's checker gives prop `p` of the exported `Row`, normalized as src/oracle.ts
// normalizes it. One program holds all fixtures.
import { existsSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { API } from 'typescript-beta/sync'
import { exportName, propsTypeOf } from './checker-components.ts'
import { listFixtures, PROP, type Fixture, type Golden } from './construct-suite.ts'
import { checkerNormalizer } from './oracle-checker.ts'

const suite = realpathSync(join(import.meta.dirname, '..', 'construct-suite'))
const check = process.argv.includes('--check')

const fixtures = listFixtures(suite)
if (fixtures.length === 0) throw new Error(`no fixtures under ${suite}`)

const api = new API({ cwd: suite })
// `parseConfigFile`, not options by hand: the 7.1 beta's server panics on string-valued enum options (src/oracle.ts).
const { options } = api.parseConfigFile(join(suite, 'tsconfig.json'))
const program = api.createProgram(
  fixtures.map((f) => f.file),
  options,
)
const { checker } = program.getProject()
const norm = checkerNormalizer(checker)

function capture(fixture: Fixture): Golden {
  const moduleSymbol = checker.getSymbolOfSourceFile(fixture.file)
  const exported =
    moduleSymbol && checker.getExportsOfModule(moduleSymbol).find((e: any) => exportName(checker, e) === 'Row')
  if (!exported) throw new Error(`${fixture.id}: no exported Row`)
  const propsType = propsTypeOf(checker, exported)
  const prop = propsType && checker.getPropertyOfType(propsType, PROP)
  if (!prop) throw new Error(`${fixture.id}: Row has no prop ${PROP}`)
  const type = checker.getTypeOfSymbol(prop)
  return { checker: norm(type), text: checker.typeToString(type) }
}

const stale: string[] = []
for (const fixture of fixtures) {
  const text = `${JSON.stringify(capture(fixture), null, 2)}\n`
  const current = existsSync(fixture.golden) ? readFileSync(fixture.golden, 'utf8') : undefined
  if (current === text) continue
  stale.push(fixture.id)
  if (!check) writeFileSync(fixture.golden, text)
}
program.dispose()
api.close()

if (check && stale.length > 0) {
  console.error(
    `golden files out of date (run pnpm --filter caliper-docgen-oracle construct:capture):\n  ${stale.join('\n  ')}`,
  )
  process.exit(1)
}
console.error(`${fixtures.length} fixtures, ${stale.length} ${check ? 'differ' : 'written'}`)
